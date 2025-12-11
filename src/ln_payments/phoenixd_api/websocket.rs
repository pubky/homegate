use futures_util::Stream;
use reqwest_websocket::{Message, RequestBuilderExt, WebSocket};
use std::pin::Pin;
use std::task::{Context, Poll};
use url::Url;

// TODO: Reconnect when the socket goes down.
// TODO: Turn the messages into proper objects.
// TODO: Handle errors gracefully.
// TODO: Handle reconnects gracefully.

/// Receive Payments Websocket
pub struct ReceivePaymentsWebsocket {
    websocket: WebSocket,
}

impl ReceivePaymentsWebsocket {
    /// Connect to the receive payments websocket
    ///
    /// # Arguments
    ///
    /// * `phoenixd_base_url` - The base URL of the PhoenixD API
    /// * `phoenixd_password` - The password of the PhoenixD API
    /// * `reqwest_client` - The reqwest client to use to connect to the websocket
    ///
    /// # Returns
    ///
    /// * `Result<Self, reqwest_websocket::Error>` - The receive payments websocket
    ///
    /// # Errors
    ///
    /// * `reqwest_websocket::Error` - If the websocket connection fails
    ///
    pub async fn connect(
        phoenixd_base_url: &Url,
        phoenixd_password: &str,
        reqwest_client: &reqwest::Client,
    ) -> Result<Self, reqwest_websocket::Error> {
        let url = phoenixd_base_url
            .join("/websocket")
            .expect("input is always valid");
        let response = reqwest_client
            .get(url)
            .basic_auth("", Some(&phoenixd_password))
            .upgrade()
            .send()
            .await?;
        let websocket = response.into_websocket().await?;
        Ok(Self { websocket })
    }
}

/// Receive payment_received messages from the websocket
/// Turn them into proper objects
///  "{\n    \"type\": \"payment_received\",\n    \"timestamp\": 1765463919284,\n    \"amountSat\": 100,\n    \"paymentHash\": \"9c6a1a409e753198e9eff58f8caa855fcd9fb433b91428ed21af875432608cc1\"\n}"
impl Stream for ReceivePaymentsWebsocket {
    type Item = Result<String, reqwest_websocket::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.websocket).poll_next(cx) {
            Poll::Ready(Some(Ok(message))) => {
                match message {
                    Message::Text(text) => Poll::Ready(Some(Ok(text))),
                    Message::Binary(data) => {
                        match String::from_utf8(data.to_vec()) {
                            Ok(text) => Poll::Ready(Some(Ok(text))),
                            Err(_) => {
                                // Skip invalid UTF-8 binary messages and continue polling
                                self.poll_next(cx)
                            }
                        }
                    }
                    Message::Close { .. } => Poll::Ready(None),
                    Message::Ping(_) | Message::Pong(_) => {
                        // Skip ping/pong messages and continue polling
                        self.poll_next(cx)
                    }
                }
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ln_payments::phoenixd_api::{api::PhoenixdAPI, websocket::ReceivePaymentsWebsocket};
    use futures_util::TryStreamExt;
    use url::Url;

    #[tokio::test]
    async fn test_received_payments_websocket() {
        let mut websocket = ReceivePaymentsWebsocket::connect(
            &Url::parse("http://localhost:9740").unwrap(),
            "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef",
            &reqwest::Client::new(),
        ).await.unwrap();
        println!("Websocket connected");
        while let Some(message) = websocket.try_next().await.unwrap() {
            println!("received: {:?}", message);
        }
        println!("Websocket disconnected");
    }
}
