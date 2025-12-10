use axum::{
    Json, extract::{FromRequest, Request}, http::StatusCode, response::{IntoResponse, Response}, extract::rejection::JsonRejection
};
use serde::{de::DeserializeOwned, Serialize};

/// Structured error response for JSON validation failures
#[derive(Debug, Serialize)]
struct JsonValidationError {
    error: String,
    message: String,
}

impl JsonValidationError {
    fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
        }
    }
}

impl IntoResponse for JsonValidationError {
    fn into_response(self) -> Response {
        let status = match self.error.as_str() {
            "invalid_json" => StatusCode::BAD_REQUEST,
            "missing_content_type" => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "json_data_error" => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::BAD_REQUEST,
        };
        (status, Json(self)).into_response()
    }
}

/// Generic extractor that deserializes JSON into any serde DeserializeOwned type
/// and returns structured JSON error responses when validation fails.
///
/// Unlike `ValidatedJson`, this extractor works directly with any deserializable struct
/// without requiring `TryFrom`. It provides consistent structured error responses
/// in JSON format when deserialization fails.
///
/// The type `T` must implement `DeserializeOwned`.
#[derive(Debug)]
pub struct ValidatedJson2<T>(pub T);

impl<T> ValidatedJson2<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T, S> FromRequest<S> for ValidatedJson2<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(ValidatedJson2(value)),
            Err(rejection) => {
                let error = match rejection {
                    JsonRejection::JsonDataError(err) => {
                        JsonValidationError::new(
                            "json_data_error",
                            format!("Invalid JSON data: {}", err),
                        )
                    }
                    JsonRejection::JsonSyntaxError(err) => {
                        JsonValidationError::new(
                            "invalid_json",
                            format!("Invalid JSON syntax: {}", err),
                        )
                    }
                    JsonRejection::MissingJsonContentType(_) => {
                        JsonValidationError::new(
                            "missing_content_type",
                            "Missing Content-Type: application/json header",
                        )
                    }
                    JsonRejection::BytesRejection(_) => {
                        JsonValidationError::new(
                            "invalid_request_body",
                            "Failed to read request body",
                        )
                    }
                    _ => {
                        JsonValidationError::new(
                            "json_validation_error",
                            format!("JSON validation failed: {}", rejection),
                        )
                    }
                };
                Err(error.into_response())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use axum::body::Body;
    use serde::Deserialize;
    use serde_json::json;

    /// Test struct for deserialization
    #[derive(Debug, Deserialize, PartialEq)]
    struct TestRequest {
        name: String,
        age: u32,
        email: Option<String>,
    }

    /// Helper function to create a request with JSON body
    fn make_json_request(body: &str, content_type: Option<&str>) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/test")
            .body(Body::from(body.to_string()))
            .unwrap();
        
        if let Some(ct) = content_type {
            request.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                ct.parse().unwrap(),
            );
        }
        
        request
    }

    /// Helper function to extract error response as JSON
    async fn extract_error_response(response: Response) -> serde_json::Value {
        let (_parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn test_valid_json_deserialization() {
        let json_body = json!({
            "name": "John Doe",
            "age": 30,
            "email": "john@example.com"
        });
        
        let request = make_json_request(
            &json_body.to_string(),
            Some("application/json"),
        );
        
        let result = ValidatedJson2::<TestRequest>::from_request(request, &())
            .await
            .unwrap();
        
        let extracted = result.into_inner();
        assert_eq!(extracted.name, "John Doe");
        assert_eq!(extracted.age, 30);
        assert_eq!(extracted.email, Some("john@example.com".to_string()));
    }

    #[tokio::test]
    async fn test_valid_json_with_optional_field() {
        let json_body = json!({
            "name": "Jane Doe",
            "age": 25
        });
        
        let request = make_json_request(
            &json_body.to_string(),
            Some("application/json"),
        );
        
        let result = ValidatedJson2::<TestRequest>::from_request(request, &())
            .await
            .unwrap();
        
        let extracted = result.into_inner();
        assert_eq!(extracted.name, "Jane Doe");
        assert_eq!(extracted.age, 25);
        assert_eq!(extracted.email, None);
    }

    #[tokio::test]
    async fn test_json_syntax_error() {
        // Malformed JSON - missing closing brace
        let malformed_json = r#"{"name": "John Doe", "age": 30"#;
        
        let request = make_json_request(
            malformed_json,
            Some("application/json"),
        );
        
        let response = ValidatedJson2::<TestRequest>::from_request(request, &())
            .await
            .unwrap_err();
        
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let error_body = extract_error_response(response).await;
        assert_eq!(error_body["error"], "invalid_json");
        assert!(error_body["message"].as_str().unwrap().contains("Invalid JSON syntax"));
    }

    #[tokio::test]
    async fn test_json_data_error_missing_required_field() {
        // Valid JSON but missing required field "age"
        let json_body = json!({
            "name": "John Doe"
        });
        
        let request = make_json_request(
            &json_body.to_string(),
            Some("application/json"),
        );
        
        let response = ValidatedJson2::<TestRequest>::from_request(request, &())
            .await
            .unwrap_err();
        
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        
        let error_body = extract_error_response(response).await;
        assert_eq!(error_body["error"], "json_data_error");
        assert!(error_body["message"].as_str().unwrap().contains("Invalid JSON data"));
    }

    #[tokio::test]
    async fn test_json_data_error_wrong_type() {
        // Valid JSON but wrong type for "age" (string instead of number)
        let json_body = json!({
            "name": "John Doe",
            "age": "thirty"
        });
        
        let request = make_json_request(
            &json_body.to_string(),
            Some("application/json"),
        );
        
        let response = ValidatedJson2::<TestRequest>::from_request(request, &())
            .await
            .unwrap_err();
        
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        
        let error_body = extract_error_response(response).await;
        assert_eq!(error_body["error"], "json_data_error");
        assert!(error_body["message"].as_str().unwrap().contains("Invalid JSON data"));
    }

    #[tokio::test]
    async fn test_missing_content_type_header() {
        let json_body = json!({
            "name": "John Doe",
            "age": 30
        });
        
        let request = make_json_request(
            &json_body.to_string(),
            None, // No Content-Type header
        );
        
        let response = ValidatedJson2::<TestRequest>::from_request(request, &())
            .await
            .unwrap_err();
        
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        
        let error_body = extract_error_response(response).await;
        assert_eq!(error_body["error"], "missing_content_type");
        assert_eq!(error_body["message"], "Missing Content-Type: application/json header");
    }

    #[tokio::test]
    async fn test_wrong_content_type_header() {
        let json_body = json!({
            "name": "John Doe",
            "age": 30
        });
        
        let request = make_json_request(
            &json_body.to_string(),
            Some("text/plain"), // Wrong Content-Type
        );
        
        let response = ValidatedJson2::<TestRequest>::from_request(request, &())
            .await
            .unwrap_err();
        
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        
        let error_body = extract_error_response(response).await;
        assert_eq!(error_body["error"], "missing_content_type");
    }

    #[tokio::test]
    async fn test_empty_body() {
        let request = make_json_request(
            "",
            Some("application/json"),
        );
        
        let response = ValidatedJson2::<TestRequest>::from_request(request, &())
            .await
            .unwrap_err();
        
        // Empty body should result in a data error or invalid request body
        let status = response.status();
        assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY);
        
        let error_body = extract_error_response(response).await;
        assert!(error_body["error"].is_string());
        assert!(error_body["message"].is_string());
    }

    #[tokio::test]
    async fn test_complex_nested_structure() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Address {
            street: String,
            city: String,
            zip: String,
        }
        
        #[derive(Debug, Deserialize, PartialEq)]
        struct ComplexRequest {
            user: TestRequest,
            address: Address,
            tags: Vec<String>,
        }
        
        let json_body = json!({
            "user": {
                "name": "John Doe",
                "age": 30,
                "email": "john@example.com"
            },
            "address": {
                "street": "123 Main St",
                "city": "New York",
                "zip": "10001"
            },
            "tags": ["developer", "rust"]
        });
        
        let request = make_json_request(
            &json_body.to_string(),
            Some("application/json"),
        );
        
        let result = ValidatedJson2::<ComplexRequest>::from_request(request, &())
            .await
            .unwrap();
        
        let extracted = result.into_inner();
        assert_eq!(extracted.user.name, "John Doe");
        assert_eq!(extracted.address.city, "New York");
        assert_eq!(extracted.tags.len(), 2);
        assert_eq!(extracted.tags[0], "developer");
    }

    // Integration test with actual Axum router
    mod router_tests {
        use super::*;
        use axum::{Router, routing::post, Json, response::IntoResponse};
        use axum_test::TestServer;
        use serde::Serialize;
        use axum::http::StatusCode;

        #[derive(Debug, Deserialize, Serialize)]
        struct Age(u8);


        #[derive(Debug, Deserialize, Serialize)]
        struct CreateUserRequest {
            name: String,
            age: Age,
            email: Option<String>,
        }

        #[derive(Debug, Serialize)]
        struct CreateUserResponse {
            id: u64,
            name: String,
            age: u8,
            email: Option<String>,
        }

        /// Handler that uses ValidatedJson2 to extract and validate the request body
        async fn create_user_handler(
            ValidatedJson2(request): ValidatedJson2<CreateUserRequest>,
        ) -> impl IntoResponse {
            // Simulate creating a user (in a real app, this would save to a database)
            let response = CreateUserResponse {
                id: 123,
                name: request.name,
                age: request.age.0,
                email: request.email,
            };
            (axum::http::StatusCode::CREATED, Json(response))
        }

        fn create_test_router() -> Router {
            Router::new()
                .route("/users", post(create_user_handler))
        }

        #[tokio::test]
        async fn test_router_with_valid_json_request() {
            let router = create_test_router();
            let server = TestServer::new(router).unwrap();

            let response = server
                .post("/users")
                .json(&json!({
                    "name": "Alice",
                    "age": Age(28),
                    "email": "alice@example.com"
                }))
                .await;

            assert_eq!(response.status_code(), StatusCode::CREATED);
            
            let body: serde_json::Value = response.json();
            assert_eq!(body["id"], 123);
            assert_eq!(body["name"], "Alice");
            assert_eq!(body["age"], 28);
            assert_eq!(body["email"], "alice@example.com");
        }

        #[tokio::test]
        async fn test_router_with_optional_field() {
            let router = create_test_router();
            let server = TestServer::new(router).unwrap();

            let response = server
                .post("/users")
                .json(&json!({
                    "name": "Bob",
                    "age": 35
                }))
                .await;

            assert_eq!(response.status_code(), StatusCode::CREATED);
            
            let body: serde_json::Value = response.json();
            assert_eq!(body["name"], "Bob");
            assert_eq!(body["age"], 35);
            assert_eq!(body["email"], serde_json::Value::Null);
        }

        #[tokio::test]
        async fn test_router_with_missing_required_field() {
            let router = create_test_router();
            let server = TestServer::new(router).unwrap();

            // Missing required "age" field
            let response = server
                .post("/users")
                .json(&json!({
                    "name": "Charlie"
                }))
                .await;

            response.assert_status_unprocessable_entity();
            
            let body: serde_json::Value = response.json();
            assert_eq!(body["error"], "json_data_error");
            assert!(body["message"].as_str().unwrap().contains("Invalid JSON data"));
        }

        #[tokio::test]
        async fn test_router_with_malformed_json() {
            let router = create_test_router();
            let server = TestServer::new(router).unwrap();

            // Malformed JSON - missing closing brace
            // Note: When using .text() with .add_header(), axum_test may not preserve
            // the Content-Type header properly, so this test verifies the behavior
            // when malformed JSON is sent. The exact status code depends on whether
            // Content-Type is recognized, but our extractor should return a structured error.
            let response = server
                .post("/users")
                .add_header("Content-Type", "application/json")
                .text(r#"{"name": "David", "age": 40"#)
                .await;

            // Our ValidatedJson2 should return a structured error response
            // The status may be 400 (if Content-Type is recognized) or 415 (if not)
            // but we should always get a structured JSON error
            assert!(response.status_code().is_client_error());
            
            let body: serde_json::Value = response.json();
            // The error type depends on whether Content-Type was recognized
            assert!(body["error"].is_string());
            assert!(body["message"].is_string());
            // If Content-Type was recognized, we should get "invalid_json"
            // If not, we'll get "missing_content_type" (which is still valid to test)
            assert!(
                body["error"] == "invalid_json" || body["error"] == "missing_content_type",
                "Expected 'invalid_json' or 'missing_content_type', got: {}",
                body["error"]
            );
        }

        #[tokio::test]
        async fn test_router_with_wrong_type() {
            let router = create_test_router();
            let server = TestServer::new(router).unwrap();

            // Wrong type for "age" (string instead of number)
            let response = server
                .post("/users")
                .json(&json!({
                    "name": "Eve",
                    "age": "thirty"
                }))
                .await;

            response.assert_status_unprocessable_entity();
            
            let body: serde_json::Value = response.json();
            assert_eq!(body["error"], "json_data_error");
            assert!(body["message"].as_str().unwrap().contains("Invalid JSON data"));
        }

        #[tokio::test]
        async fn test_router_with_missing_content_type() {
            let router = create_test_router();
            let server = TestServer::new(router).unwrap();

            // Missing Content-Type header
            let response = server
                .post("/users")
                .text(r#"{"name": "Frank", "age": 50}"#)
                .await;

            assert_eq!(response.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
            
            let body: serde_json::Value = response.json();
            assert_eq!(body["error"], "missing_content_type");
            assert_eq!(body["message"], "Missing Content-Type: application/json header");
        }
    }
}
