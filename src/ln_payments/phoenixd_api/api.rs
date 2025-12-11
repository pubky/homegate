use reqwest_websocket::{RequestBuilderExt};
use serde;
use serde_json::json;


/// PhoenixD is a Lightning Network node that can be used to accept Lightning Network payments.
///
/// API docs: https://phoenix.acinq.co/server/api
///
/// Docker run: `docker run -p 9740:9740 -d acinq/phoenixd:latest`
///
/// Admin api: `http://localhost:9740`
///
use url::Url;

use crate::ln_payments::phoenixd_api::websocket::ReceivePaymentsWebsocket;

/// Invoice Response
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceResponse {
    /// Invoice Amount in sat
    pub amount_sat: u64,
    /// Payment Hash (hex encoded id)
    pub payment_hash: String,
    /// Bolt11
    #[serde(rename = "serialized")]
    pub bolt11_invoice: String,
}

/// Find Incoming Response
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetIncomingInvoiceResponse {
    /// Payment Hash
    pub payment_hash: String,
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
    /// Fees
    pub fees: u64,
    /// Completed at
    pub completed_at: Option<u64>,
    /// Time created
    pub created_at: u64,
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
    ) -> Result<InvoiceResponse, reqwest::Error> {
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
        let invoice_response = response.json::<InvoiceResponse>().await?;
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
        payment_hash: &str,
    ) -> Result<GetIncomingInvoiceResponse, reqwest::Error> {
        let url = self
            .base_url
            .join("/payments/incoming/")
            .expect("input is always valid")
            .join(payment_hash)
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

    /// Syntactic sugar to build a websocket connection to the receive payments websocket
    pub async fn received_payments_websocket(&self) -> Result<ReceivePaymentsWebsocket, reqwest_websocket::Error> {
        Ok(ReceivePaymentsWebsocket::connect(&self.base_url, &self.password, &self.http_client).await?)
    }
}

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt;

    use super::*;

    #[tokio::test]
    async fn test_new_invoice() {
        let api = PhoenixdAPI::new(
            &Url::parse("http://localhost:9740").unwrap(),
            "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef",
        );
        let invoice = api
            .create_invoice(100, "Test Invoice", 60 * 10)
            .await
            .unwrap();
        println!("Invoice: {:?}", invoice);
        assert_eq!(invoice.amount_sat, 100);
        assert!(!invoice.payment_hash.is_empty());
        assert!(!invoice.bolt11_invoice.is_empty());
    }

    #[tokio::test]
    async fn test_get_invoice() {
        let api = PhoenixdAPI::new(
            &Url::parse("http://localhost:9740").unwrap(),
            "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef",
        );
        let invoice = api
            .create_invoice(1000, "Test Invoice", 60 * 10)
            .await
            .unwrap();
        println!("Invoice: {:?}", invoice);
        assert_eq!(invoice.amount_sat, 1000);
        assert!(!invoice.payment_hash.is_empty());
        assert!(!invoice.bolt11_invoice.is_empty());

        let invoice_get = api.get_invoice(&invoice.payment_hash).await.unwrap();
        println!("Invoice: {:?}", invoice_get);
        assert_eq!(invoice_get.payment_hash, invoice.payment_hash);
        assert_eq!(invoice_get.description, "Test Invoice");
        assert_eq!(invoice_get.invoice, invoice.bolt11_invoice);
        assert_eq!(invoice_get.is_paid, false);
        assert_eq!(invoice_get.received_sat, 0);
        assert_eq!(invoice_get.fees, 0);
        assert_eq!(invoice_get.completed_at, None);
    }
}
