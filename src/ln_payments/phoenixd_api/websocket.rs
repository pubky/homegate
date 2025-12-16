use futures_util::Stream;
use reqwest_websocket::{Message, RequestBuilderExt, WebSocket};
use serde_json;
use std::pin::Pin;
use std::task::{Context, Poll};
use url::Url;

/// Payment received event
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentReceivedEvent {
    /// Amount of the payment received in satoshis
    pub amount_sat: u64,
    /// Payment hash of the payment received
    pub payment_hash: String,
}

/// Internal struct for deserializing websocket messages
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebsocketMessage {
    #[serde(rename = "type")]
    message_type: String,
}

/// Receive Payments Websocket
pub struct ReceivePaymentsWebsocket {
    websocket: WebSocket,
    closed: bool,
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
        Ok(Self {
            websocket,
            closed: false,
        })
    }

    /// Close the websocket connection
    pub async fn close(self) -> Result<(), reqwest_websocket::Error> {
        self.websocket
            .close(reqwest_websocket::CloseCode::Normal, None)
            .await
    }
}

impl Stream for ReceivePaymentsWebsocket {
    type Item = Result<PaymentReceivedEvent, Box<dyn std::error::Error + Send + Sync>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.websocket).poll_next(cx) {
                Poll::Ready(Some(Ok(Message::Text(text)))) => {
                    let message = match serde_json::from_str::<WebsocketMessage>(&text) {
                        Ok(msg) => msg,
                        Err(e) => {
                            return Poll::Ready(Some(Err(Box::new(e))));
                        }
                    };
                    if message.message_type != "payment_received" {
                        // Not a payment_received message, skip it
                        continue;
                    }

                    let actual = match serde_json::from_str::<PaymentReceivedEvent>(&text) {
                        Ok(msg) => msg,
                        Err(e) => {
                            return Poll::Ready(Some(Err(Box::new(e))));
                        }
                    };

                    return Poll::Ready(Some(Ok(actual)));
                }
                Poll::Ready(Some(Ok(Message::Binary(_)))) => {
                    // Skip binary messages
                    continue;
                }
                Poll::Ready(Some(Ok(Message::Close { .. }))) => {
                    // Connection closed
                    return Poll::Ready(None);
                }
                Poll::Ready(Some(Ok(Message::Ping(_)))) => {
                    // Ping messages are handled by the websocket library
                    continue;
                }
                Poll::Ready(Some(Ok(Message::Pong(_)))) => {
                    // Pong messages are handled by the websocket library
                    continue;
                }
                Poll::Ready(Some(Err(e))) => {
                    // Websocket error
                    println!("Websocket error: {:?}", e);
                    self.closed = true;
                    println!("Websocket closed");
                    return Poll::Ready(Some(Err(Box::new(e))));
                }
                Poll::Ready(None) => {
                    // Stream ended, set closed to true
                    self.closed = true;
                    println!("Websocket closed");
                    return Poll::Ready(None);
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ln_payments::phoenixd_api::api::PhoenixdAPI;
    use crate::ln_payments::phoenixd_api::websocket::{
        PaymentReceivedEvent, ReceivePaymentsWebsocket,
    };
    use axum::{
        Router, extract::ws::WebSocketUpgrade, http::HeaderMap, response::IntoResponse,
        routing::get,
    };
    use axum_server::Handle;
    use base64::Engine;
    use futures_util::{SinkExt, StreamExt, TryStreamExt};
    use std::net::TcpListener;
    use std::sync::Arc;
    use url::Url;

    /// Mock websocket server that handles HTTP upgrade and sends payment events
    struct MockWebsocketServer {
        handle: Handle,
        port: u16,
    }

    impl MockWebsocketServer {
        /// Start a mock websocket server on a random port
        async fn start(password: String, messages: Vec<PaymentReceivedEvent>) -> (Self, Url) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let base_url = Url::parse(&format!("http://127.0.0.1:{}", port)).unwrap();

            let messages = Arc::new(messages);
            let password = Arc::new(password);

            let app = Router::new().route(
                "/websocket",
                get(move |headers: HeaderMap, ws: WebSocketUpgrade| async move {
                    // Verify basic auth
                    let auth_header = headers.get("authorization");
                    let expected_auth = format!(
                        "Basic {}",
                        base64::engine::general_purpose::STANDARD.encode(format!(":{}", password))
                    );

                    if let Some(auth) = auth_header {
                        if auth.to_str().unwrap() != expected_auth {
                            return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized")
                                .into_response();
                        }
                    } else {
                        return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized")
                            .into_response();
                    }

                    let messages = messages.clone();
                    ws.on_upgrade(move |socket| async move {
                        let (mut sender, _receiver) = socket.split();

                        // Send mock messages with the required "type" field
                        for msg in messages.iter() {
                            let json = serde_json::json!({
                                "type": "payment_received",
                                "amountSat": msg.amount_sat,
                                "paymentHash": msg.payment_hash,
                            });
                            let json_str = serde_json::to_string(&json).unwrap();
                            if sender
                                .send(axum::extract::ws::Message::Text(json_str.into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                            // Small delay between messages
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                        }
                    })
                }),
            );

            let handle = Handle::new();
            let handle_clone = handle.clone();
            tokio::spawn(async move {
                axum_server::from_tcp(listener)
                    .handle(handle_clone)
                    .serve(app.into_make_service())
                    .await
                    .unwrap();
            });

            // Give the server a moment to start
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            (Self { handle, port }, base_url)
        }
    }

    #[tokio::test]
    async fn test_received_payments_websocket() {
        let password = "test-password-123";
        let mock_events = vec![
            PaymentReceivedEvent {
                amount_sat: 100,
                payment_hash: "hash1".to_string(),
            },
            PaymentReceivedEvent {
                amount_sat: 200,
                payment_hash: "hash2".to_string(),
            },
        ];

        let (_server, base_url) =
            MockWebsocketServer::start(password.to_string(), mock_events.clone()).await;

        let mut websocket =
            ReceivePaymentsWebsocket::connect(&base_url, password, &reqwest::Client::new())
                .await
                .unwrap();

        // Collect received messages
        let mut received = Vec::new();
        let mut timeout_counter = 0;
        loop {
            tokio::select! {
                result = websocket.try_next() => {
                    match result {
                        Ok(Some(event)) => {
                            received.push(event);
                            if received.len() >= mock_events.len() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => panic!("Error receiving message: {:?}", e),
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                    timeout_counter += 1;
                    if timeout_counter > 50 {
                        // 5 second timeout
                        break;
                    }
                }
            }
        }

        assert_eq!(received.len(), mock_events.len());
        for (i, event) in received.iter().enumerate() {
            assert_eq!(event.amount_sat, mock_events[i].amount_sat);
            assert_eq!(event.payment_hash, mock_events[i].payment_hash);
        }
    }

    #[tokio::test]
    async fn test_received_payments_websocket_close() {
        let mut websocket = ReceivePaymentsWebsocket::connect(
            &Url::parse("http://localhost:9740").unwrap(),
            "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef",
            &reqwest::Client::new(),
        )
        .await
        .unwrap();

        let api = PhoenixdAPI::new(
            &Url::parse("http://localhost:9740").unwrap(),
            "a1fabd1a106e7283a1e5b6e4f0dd58a67905cde51297465c7bf3658317d14eef",
        );

        let invoice = api
            .create_invoice(100, "Test Invoice", 60 * 10)
            .await
            .unwrap();
        println!("Invoice: {:?}", invoice.bolt11_invoice);

        match websocket.try_next().await {
            Ok(Some(event)) => {
                println!("Received event: {:?}", event);
            }
            Ok(None) => {
                println!("No event received");
            }
            Err(e) => {
                println!("Error receiving event: {:?}", e);
            }
        }

        println!("Websocket closed: {:?}", websocket.closed);
    }
}
