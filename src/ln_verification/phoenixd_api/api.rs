//! PhoenixD is a Lightning Network node that can be used to accept Lightning Network payments.
//!
//! API docs: https://phoenix.acinq.co/server/api
//!
//! Docker run: `docker run -p 9740:9740 -d acinq/phoenixd:latest`
//!
//! Admin api: `http://localhost:9740`
//!

use chrono::{DateTime, Utc};
use serde::{self, Deserialize};
use serde_json::json;
use url::Url;
#[cfg(test)]
use wiremock::MockServer;

use crate::ln_verification::{payment_hash::PaymentHash, phoenixd_api::{WebsocketError, websocket::ReceivePaymentsWebsocket}};

/// Helper function to deserialize a u64 timestamp in milliseconds to DateTime<Utc>
fn deserialize_timestamp_millis<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let millis = u64::deserialize(deserializer)?;
    DateTime::<Utc>::from_timestamp_millis(millis as i64)
        .ok_or_else(|| serde::de::Error::custom(format!("Invalid timestamp: {}", millis)))
}

/// Helper function to deserialize an Option<u64> timestamp in milliseconds to Option<DateTime<Utc>>
fn deserialize_option_timestamp_millis<'de, D>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let millis: Option<u64> = Option::deserialize(deserializer)?;
    match millis {
        Some(ms) => DateTime::<Utc>::from_timestamp_millis(ms as i64)
            .ok_or_else(|| serde::de::Error::custom(format!("Invalid timestamp: {}", ms)))
            .map(Some),
        None => Ok(None),
    }
}

/// Invoice Response
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvoiceResponse {
    /// Invoice Amount in sat
    pub amount_sat: u64,
    /// Payment Hash (hex encoded id)
    pub payment_hash: PaymentHash,
    /// Bolt11
    #[serde(rename = "serialized")]
    pub bolt11_invoice: String,
}

/// Find Incoming Response
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetIncomingInvoiceResponse {
    /// Payment Hash
    pub payment_hash: PaymentHash,
    /// Preimage
    pub preimage: String,
    /// External Id
    pub external_id: Option<String>,
    /// Description
    pub description: String,
    /// Bolt11 invoice
    pub invoice: String,
    /// Paid flag
    pub is_paid: bool,
    /// Sats received
    pub received_sat: u64,
    /// Requested satoshis
    pub requested_sat: u64,
    /// Fees
    pub fees: u64,
    /// Completed at timestamp in milliseconds
    #[serde(default, deserialize_with = "deserialize_option_timestamp_millis")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Time created timestamp in milliseconds
    #[serde(deserialize_with = "deserialize_timestamp_millis")]
    pub created_at: DateTime<Utc>,
    /// Expired flag
    pub is_expired: bool,
    /// Expires at timestamp in milliseconds
    #[serde(deserialize_with = "deserialize_timestamp_millis")]
    pub expires_at: DateTime<Utc>,
}

/// PhoenixD API
///
/// # Arguments
///
/// * `base_url` - The base URL of the PhoenixD API - Usually something like "http://localhost:9740"
/// * `password` - The password of the PhoenixD API - See `~/.phoenix/phoenix.conf`
///
/// # Returns
///
/// * `PhoenixdAPI` - The PhoenixD API
///
/// # Errors
///
/// * `reqwest::Error` - If the request fails
///
#[derive(Clone, Debug)]
pub struct PhoenixdAPI {
    http_client: reqwest::Client,
    base_url: Url,
    password: String,
}

impl PhoenixdAPI {
    pub fn new(base_url: &Url, password: &str) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            base_url: base_url.clone(),
            password: password.to_owned(),
        }
    }

    /// Creates a new invoice for the given amount, description, and expiry seconds
    ///
    /// # Arguments
    ///
    /// * `amount_satoshis` - The amount of satoshis to invoice
    /// * `description` - The description of the invoice
    /// * `expiry_seconds` - The expiry seconds of the invoice
    ///
    /// # Returns
    ///
    /// * `InvoiceResponse` - The response from the create invoice endpoint
    ///
    /// # Errors
    ///
    /// * `reqwest::Error` - If the request fails
    ///
    pub async fn create_invoice(
        &self,
        amount_satoshis: u64,
        description: &str,
        expiry_seconds: u64,
    ) -> Result<CreateInvoiceResponse, reqwest::Error> {
        let url = self
            .base_url
            .join("/createinvoice")
            .expect("input is always valid");
        let response = self
            .http_client
            .post(url)
            .basic_auth("", Some(&self.password))
            .form(&json!({
                "amountSat": amount_satoshis,
                "description": description,
                "expirySeconds": expiry_seconds,
            }))
            .send()
            .await?;
        let response = response.error_for_status()?;
        let invoice_response = response.json::<CreateInvoiceResponse>().await?;
        Ok(invoice_response)
    }

    /// Gets an invoice by payment hash
    ///
    /// # Arguments
    ///
    /// * `payment_hash` - The payment hash of the invoice
    ///
    /// # Returns
    ///
    /// * `GetIncomingInvoiceResponse` - The response from the get invoice endpoint
    ///
    /// # Errors
    ///
    /// * `reqwest::Error` - If the request fails
    ///
    pub async fn get_invoice(
        &self,
        payment_hash: &PaymentHash,
    ) -> Result<GetIncomingInvoiceResponse, reqwest::Error> {
        let url = self
            .base_url
            .join("/payments/incoming/")
            .expect("input is always valid")
            .join(payment_hash.as_str())
            .expect("input is always valid");
        let response = self
            .http_client
            .get(url)
            .basic_auth("", Some(&self.password))
            .send()
            .await?;
        let response = response.error_for_status()?;
        let invoice_response = response.json::<GetIncomingInvoiceResponse>().await?;
        Ok(invoice_response)
    }

    /// Lists paid incoming invoices.
    /// This call is paginated.
    ///
    /// # Arguments
    ///
    /// * `from` - The date and time to filter invoices from
    /// * `offset` - The offset to start from (default is 0)
    /// * `limit` - The limit of invoices to return (default is 20)
    /// * `all` - Whether to return all or only paid invoices (default is false)
    ///
    /// # Returns
    ///
    /// * `Vec<GetIncomingInvoiceResponse>` - The list of paid incoming invoices
    ///
    /// # Errors
    ///
    /// * `reqwest::Error` - If the request fails
    ///
    pub async fn list_paid_invoices(
        &self,
        from: DateTime<Utc>,
        limit: Option<u64>,
        offset: Option<u64>,
        all: bool,
    ) -> Result<Vec<GetIncomingInvoiceResponse>, reqwest::Error> {
        let url = self
            .base_url
            .join("/payments/incoming")
            .expect("input is always valid");

        let mut query_params = vec![
            ("from", from.timestamp_millis().to_string()),
            ("offset", offset.unwrap_or(0).to_string()),
            ("limit", limit.unwrap_or(20).to_string()),
        ];
        if all {
            query_params.push(("all", "true".to_string()));
        }

        let response = self
            .http_client
            .get(url)
            .query(&query_params)
            .basic_auth("", Some(&self.password))
            .send()
            .await?;
        let response = response.error_for_status()?;
        let invoice_response = response.json::<Vec<GetIncomingInvoiceResponse>>().await?;
        Ok(invoice_response)
    }

    /// Syntactic sugar to build a websocket connection to receive payment events
    pub async fn received_payments_websocket(
        &self,
    ) -> Result<ReceivePaymentsWebsocket, WebsocketError> {
        Ok(
            ReceivePaymentsWebsocket::connect(&self.base_url, &self.password, &self.http_client)
                .await?,
        )
    }

    #[cfg(test)]
    pub fn test() -> Self {
        Self::new(&Url::parse("http://localhost:9740").unwrap(), "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef")
    }
}

#[cfg(test)]
mod tests {
    use chrono::Days;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{BasicAuthMatcher, method, path},
    };

    use super::*;

    #[tokio::test]
    async fn test_new_invoice() {
        let password = "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef";
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/createinvoice"))
            .and(BasicAuthMatcher::from_credentials("", password))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "amountSat": 100,
                "paymentHash": "bb4b823baf2ed506c5c8679e5887499aff7097e063d91eaa2d135fe35c88d289",
                "serialized": "lnbc1u1p55qgw4pp5hd9cywa09m2sd3wgv7093p6fntlhp9lqv0v3a23dzd07xhyg62yscqzyssp5a4qlh8thyzdv22vl2wyjaz90axeaayxfetz5c5palxfy56dcjczs9q7sqqqqqqqqqqqqqqqqqqqsqqqqqysgqdq523jhxapqf9h8vmmfvdjsmqz9gxqzjcrzjqwryaup9lh50kkranzgcdnn2fgvx390wgj5jd07rwr3vxeje0glcll6h0fz6dguxyqqqqqlgqqqqqeqqjqm20sevvla34cg5gevrzvw47kc7halr0szgamung28wutm8pj4wmygafts8z394gpxus5ap084uaz6cx8mccfes9yqlxg59zf4mwh4ygqkyl75x",
            })))
            .expect(1)
            .mount(&mock_server).await;

        let api = PhoenixdAPI::new(&mock_server.uri().parse().unwrap(), password);
        let invoice = api
            .create_invoice(100, "Test Invoice", 60 * 10)
            .await
            .unwrap();
        assert_eq!(invoice.amount_sat, 100);
        assert_eq!(invoice.payment_hash, PaymentHash::new("bb4b823baf2ed506c5c8679e5887499aff7097e063d91eaa2d135fe35c88d289").unwrap());
        assert!(!invoice.bolt11_invoice.is_empty());

        mock_server.verify().await;
    }

    #[tokio::test]
    async fn test_get_invoice() {
        let password = "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef";
        let payment_hash = "bb4b823baf2ed506c5c8679e5887499aff7097e063d91eaa2d135fe35c88d289";
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/payments/incoming/{}", payment_hash).as_str()))
            .and(BasicAuthMatcher::from_credentials("", password))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "paymentHash": "bb4b823baf2ed506c5c8679e5887499aff7097e063d91eaa2d135fe35c88d289",
                "preimage": "02000000000101571874492f5318824956365707b307b49988488862022b74e16219b306900000000000000000000000000000000000000000000000000000000000000000000000000",
                "externalId": "test-external-id",
                "description": "Test Invoice",
                "invoice": "lnbc1u1p55qgw4pp5hd9cywa09m2sd3wgv7093p6fntlhp9lqv0v3a23dzd07xhyg62yscqzyssp5a4qlh8thyzdv22vl2wyjaz90axeaayxfetz5c5palxfy56dcjczs9q7sqqqqqqqqqqqqqqqqqqqsqqqqqysgqdq523jhxapqf9h8vmmfvdjsmqz9gxqzjcrzjqwryaup9lh50kkranzgcdnn2fgvx390wgj5jd07rwr3vxeje0glcll6h0fz6dguxyqqqqqlgqqqqqeqqjqm20sevvla34cg5gevrzvw47kc7halr0szgamung28wutm8pj4wmygafts8z394gpxus5ap084uaz6cx8mccfes9yqlxg59zf4mwh4ygqkyl75x",
                "isPaid": false,
                "receivedSat": 0,
                "fees": 1,  
                "completedAt": null,
                "createdAt": 1765463919,
                "isExpired": false,
                "expiresAt": 1765463919,
                "requestedSat": 100,
            })))
            .expect(1)
            .mount(&mock_server).await;

        let api = PhoenixdAPI::new(&mock_server.uri().parse().unwrap(), password);

        let invoice_get = api.get_invoice(&PaymentHash::new(payment_hash).unwrap()).await.unwrap();
        assert_eq!(invoice_get.payment_hash, PaymentHash::new(payment_hash).unwrap());
        assert_eq!(invoice_get.description, "Test Invoice");
        assert_eq!(
            invoice_get.invoice,
            "lnbc1u1p55qgw4pp5hd9cywa09m2sd3wgv7093p6fntlhp9lqv0v3a23dzd07xhyg62yscqzyssp5a4qlh8thyzdv22vl2wyjaz90axeaayxfetz5c5palxfy56dcjczs9q7sqqqqqqqqqqqqqqqqqqqsqqqqqysgqdq523jhxapqf9h8vmmfvdjsmqz9gxqzjcrzjqwryaup9lh50kkranzgcdnn2fgvx390wgj5jd07rwr3vxeje0glcll6h0fz6dguxyqqqqqlgqqqqqeqqjqm20sevvla34cg5gevrzvw47kc7halr0szgamung28wutm8pj4wmygafts8z394gpxus5ap084uaz6cx8mccfes9yqlxg59zf4mwh4ygqkyl75x"
        );
        assert_eq!(invoice_get.is_paid, false);
        assert_eq!(invoice_get.received_sat, 0);
        assert_eq!(invoice_get.fees, 1);
        assert_eq!(invoice_get.completed_at, None);
        assert_eq!(
            invoice_get.created_at,
            DateTime::<Utc>::from_timestamp_millis(1765463919).unwrap()
        );
        assert_eq!(
            invoice_get.expires_at,
            DateTime::<Utc>::from_timestamp_millis(1765463919).unwrap()
        );
        assert_eq!(invoice_get.requested_sat, 100);
    }

    #[tokio::test]
    async fn test_list_paid_invoices() {
        let password = "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef";
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/payments/incoming"))
            .and(BasicAuthMatcher::from_credentials("", password))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {
                    "paymentHash": "bb4b823baf2ed506c5c8679e5887499aff7097e063d91eaa2d135fe35c88d289",
                    "preimage": "02000000000101571874492f5318824956365707b307b49988488862022b74e16219b306900000000000000000000000000000000000000000000000000000000000000000000000000",
                    "externalId": "test-external-id",
                    "description": "Test Invoice",
                    "invoice": "lnbc1u1p55qgw4pp5hd9cywa09m2sd3wgv7093p6fntlhp9lqv0v3a23dzd07xhyg62yscqzyssp5a4qlh8thyzdv22vl2wyjaz90axeaayxfetz5c5palxfy56dcjczs9q7sqqqqqqqqqqqqqqqqqqqsqqqqqysgqdq523jhxapqf9h8vmmfvdjsmqz9gxqzjcrzjqwryaup9lh50kkranzgcdnn2fgvx390wgj5jd07rwr3vxeje0glcll6h0fz6dguxyqqqqqlgqqqqqeqqjqm20sevvla34cg5gevrzvw47kc7halr0szgamung28wutm8pj4wmygafts8z394gpxus5ap084uaz6cx8mccfes9yqlxg59zf4mwh4ygqkyl75x",
                    "isPaid": true,
                    "receivedSat": 100,
                    "fees": 1,
                    "completedAt": 1765463919,
                    "createdAt": 1765463919,
                    "isExpired": false,
                    "expiresAt": 1765463919,
                    "requestedSat": 100,

                }
            ])))
            .expect(1)
            .mount(&mock_server).await;

        let api = PhoenixdAPI::new(&mock_server.uri().parse().unwrap(), password);

        let invoices = api
            .list_paid_invoices(
                Utc::now().checked_sub_days(Days::new(7)).unwrap(),
                None,
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(invoices.len(), 1);
        assert_eq!(
            invoices[0].payment_hash,
            PaymentHash::new("bb4b823baf2ed506c5c8679e5887499aff7097e063d91eaa2d135fe35c88d289").unwrap()
        );
        assert_eq!(invoices[0].description, "Test Invoice");
        assert_eq!(invoices[0].is_paid, true);
        assert_eq!(invoices[0].received_sat, 100);
        assert_eq!(invoices[0].fees, 1);
        assert_eq!(
            invoices[0].completed_at,
            Some(DateTime::<Utc>::from_timestamp_millis(1765463919).unwrap())
        );
        assert_eq!(
            invoices[0].created_at,
            DateTime::<Utc>::from_timestamp_millis(1765463919).unwrap()
        );

        mock_server.verify().await;
    }

    #[tokio::test]
    async fn test_list_paid_invoices_real() {
        let password = "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef";
        let base_url = Url::parse("http://localhost:9740").unwrap();
        let api = PhoenixdAPI::new(&base_url, password);

        let invoices = api
            .list_paid_invoices(
                Utc::now().checked_sub_days(Days::new(7)).unwrap(),
                Some(2),
                None,
                false,
            )
            .await
            .unwrap();

        println!("Invoices: {:?}", invoices.len());
        for invoice in invoices {
            println!("Invoice: {:?}", invoice);
        }
    }
}
