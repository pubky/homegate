use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use url::Url;

use crate::sms_verification::{Code, PhoneNumber};

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PreludeErrorCode {
    RegionBlockedByCustomer,
    InvalidPhoneNumber,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Debug)]
struct PreludeErrorResponse {
    code: PreludeErrorCode,
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    request_id: Option<String>,
}

#[derive(Debug)]
pub enum PreludeError {
    RateLimited { retry_after: Option<u64> },
    RegionBlocked,
    InvalidPhoneNumber,
    RequestFailed(reqwest::Error),
}

impl PreludeError {
    /// Check if response is a 429, extract retry-after header if present and return PreludeApi error.
    /// Otherwise, pass back the same reqwest::Response
    /// All Prelude errors - https://docs.prelude.so/introduction/errors
    pub async fn from_response(response: reqwest::Response) -> Result<reqwest::Response, Self> {
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            return Err(PreludeError::RateLimited { retry_after });
        }

        if !response.status().is_success() {
            let status = response.status();
            let status_error = response
                .error_for_status_ref()
                .expect_err("Already checked !is_success()");
            let body_text = response
                .text()
                .await
                .unwrap_or_else(|_| "<unable to read body>".to_string());

            match serde_json::from_str::<PreludeErrorResponse>(&body_text) {
                Ok(error_response) => {
                    tracing::error!(
                        status = %status,
                        code = ?error_response.code,
                        message = %error_response.message,
                        error_type = %error_response.error_type,
                        request_id = ?error_response.request_id,
                    );
                    match error_response.code {
                        PreludeErrorCode::RegionBlockedByCustomer => {
                            return Err(PreludeError::RegionBlocked);
                        }
                        PreludeErrorCode::InvalidPhoneNumber => {
                            return Err(PreludeError::InvalidPhoneNumber);
                        }
                        _ => {}
                    }
                }
                Err(_) => {
                    tracing::info!(
                        status = %status,
                        body = %body_text,
                        "Prelude API non-2xx response (unable to parse as JSON)"
                    );
                }
            }
            return Err(PreludeError::RequestFailed(status_error));
        }

        Ok(response)
    }
}

impl std::fmt::Display for PreludeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreludeError::RateLimited { .. } => write!(f, "Rate limit exceeded"),
            PreludeError::RequestFailed(e) => write!(f, "Request failed: {}", e),
            PreludeError::RegionBlocked => write!(f, "Region blocked for given phone number"),
            PreludeError::InvalidPhoneNumber => write!(
                f,
                "The provided phone number is invalid. Provide a valid E.164 phone number."
            ),
        }
    }
}

impl std::error::Error for PreludeError {}

impl From<reqwest::Error> for PreludeError {
    fn from(e: reqwest::Error) -> Self {
        PreludeError::RequestFailed(e)
    }
}

/// Caller of Prelude's v2 API. Ref: https://docs.prelude.so/verify/v2/api-reference/
#[derive(Clone, Debug)]
pub struct PreludeAPI {
    http_client: reqwest::Client,
    api_key: String,
    base_url: Url,
}

#[derive(Serialize)]
struct Target {
    #[serde(rename = "type")]
    target_type: String,
    value: String,
}

#[derive(Serialize)]
struct Signals {
    ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,
}

#[derive(Serialize)]
struct PreludeCreateVerificationRequest {
    target: Target,
    signals: Signals,
    #[serde(skip_serializing_if = "Option::is_none")]
    dispatch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreludeBlockedReason {
    /// The signature of the SDK signals is expired. They should be sent within the hour following their collection.
    ExpiredSignature,
    /// The phone number is part of the configured block list.
    InBlockList,
    /// The phone number is not a valid line number (e.g. landline).
    InvalidPhoneLine,
    /// The phone number is not a valid phone number (e.g. unallocated range).
    InvalidPhoneNumber,
    /// The signature of the SDK signals is invalid.
    InvalidSignature,
    /// The phone number has made too many verification attempts.
    RepeatedAttempts,
    /// The verification attempt was deemed suspicious by the anti-fraud system.
    Suspicious,
    /// Prelude API returned a blocked reason we don't recognise.
    Unknown(String),
}

impl<'de> serde::Deserialize<'de> for PreludeBlockedReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = Option::<String>::deserialize(deserializer)?;
        match s.as_deref() {
            None => Ok(PreludeBlockedReason::Unknown("absent".to_string())),
            Some("expired_signature") => Ok(PreludeBlockedReason::ExpiredSignature),
            Some("in_block_list") => Ok(PreludeBlockedReason::InBlockList),
            Some("invalid_phone_line") => Ok(PreludeBlockedReason::InvalidPhoneLine),
            Some("invalid_phone_number") => Ok(PreludeBlockedReason::InvalidPhoneNumber),
            Some("invalid_signature") => Ok(PreludeBlockedReason::InvalidSignature),
            Some("repeated_attempts") => Ok(PreludeBlockedReason::RepeatedAttempts),
            Some("suspicious") => Ok(PreludeBlockedReason::Suspicious),
            Some(other) => Ok(PreludeBlockedReason::Unknown(other.to_string())),
        }
    }
}

impl serde::Serialize for PreludeBlockedReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            PreludeBlockedReason::Unknown(raw) => serializer.serialize_str(raw),
            other => serializer.serialize_str(&other.to_string()),
        }
    }
}

impl Default for PreludeBlockedReason {
    fn default() -> Self {
        PreludeBlockedReason::Unknown("absent".to_string())
    }
}

impl std::fmt::Display for PreludeBlockedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreludeBlockedReason::ExpiredSignature => write!(f, "expired_signature"),
            PreludeBlockedReason::InBlockList => write!(f, "in_block_list"),
            PreludeBlockedReason::InvalidPhoneLine => write!(f, "invalid_phone_line"),
            PreludeBlockedReason::InvalidPhoneNumber => write!(f, "invalid_phone_number"),
            PreludeBlockedReason::InvalidSignature => write!(f, "invalid_signature"),
            PreludeBlockedReason::RepeatedAttempts => write!(f, "repeated_attempts"),
            PreludeBlockedReason::Suspicious => write!(f, "suspicious"),
            PreludeBlockedReason::Unknown(s) => write!(f, "unknown({})", s),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PreludeCreateVerificationResponse {
    Success {
        id: String,
    },
    Retry {
        id: String,
    },
    Blocked {
        id: String,
        #[serde(default)]
        reason: PreludeBlockedReason,
    },
}

#[derive(Serialize)]
struct PreludeCheckCodeRequest {
    target: Target,
    code: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PreludeCheckCodeResponse {
    Success {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    Failure {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    ExpiredOrNotFound {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
}

impl PreludeAPI {
    pub fn new(base_url: &Url, api_key: &str) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            api_key: api_key.to_owned(),
            base_url: base_url.clone(),
        }
    }

    /// Creates a verification request for the given phone number
    pub async fn create_verification(
        &self,
        phone_number: &PhoneNumber,
        ip_address: IpAddr,
        user_agent: Option<String>,
        dispatch_id: Option<String>,
    ) -> Result<PreludeCreateVerificationResponse, PreludeError> {
        let request_body = PreludeCreateVerificationRequest {
            target: Target {
                target_type: "phone_number".to_string(),
                value: phone_number.to_string(),
            },
            signals: Signals {
                ip: ip_address.to_string(),
                user_agent,
            },
            dispatch_id,
        };

        let url = self
            .base_url
            .join("v2/verification")
            .expect("Failed to join URL path");
        let response = self
            .http_client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let response = PreludeError::from_response(response).await?;

        let verification_response = response.json::<PreludeCreateVerificationResponse>().await?;

        if let PreludeCreateVerificationResponse::Blocked {
            reason: PreludeBlockedReason::Unknown(ref raw),
            ref id,
        } = verification_response
        {
            tracing::warn!(
                prelude_id = %id,
                raw_reason = %raw,
                "Prelude returned unrecognised blocked reason"
            );
        }

        Ok(verification_response)
    }

    /// Checks a verification code for the given phone number
    pub async fn check_code(
        &self,
        phone_number: &PhoneNumber,
        code: &Code,
    ) -> Result<PreludeCheckCodeResponse, PreludeError> {
        let request_body = PreludeCheckCodeRequest {
            target: Target {
                target_type: "phone_number".to_string(),
                value: phone_number.to_string(),
            },
            code: code.to_string(),
        };

        let url = self
            .base_url
            .join("v2/verification/check")
            .expect("Failed to join URL path");
        let response = self
            .http_client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;
        let response = PreludeError::from_response(response).await?;

        let check_response = response.json::<PreludeCheckCodeResponse>().await?;

        Ok(check_response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_blocked_with_known_reason() {
        let json = r#"{"status":"blocked","id":"x","reason":"repeated_attempts"}"#;
        let resp: PreludeCreateVerificationResponse = serde_json::from_str(json).unwrap();
        assert!(matches!(
            resp,
            PreludeCreateVerificationResponse::Blocked {
                reason: PreludeBlockedReason::RepeatedAttempts,
                ..
            }
        ));
    }

    #[test]
    fn deserialize_blocked_with_unknown_reason() {
        let json = r#"{"status":"blocked","id":"x","reason":"brand_new_reason"}"#;
        let resp: PreludeCreateVerificationResponse = serde_json::from_str(json).unwrap();
        match resp {
            PreludeCreateVerificationResponse::Blocked { reason, .. } => {
                assert_eq!(
                    reason,
                    PreludeBlockedReason::Unknown("brand_new_reason".to_string())
                );
            }
            other => panic!("Expected Blocked variant, got {:?}", other),
        }
    }

    #[test]
    fn deserialize_blocked_without_reason_field() {
        let json = r#"{"status":"blocked","id":"x"}"#;
        let resp: PreludeCreateVerificationResponse = serde_json::from_str(json).unwrap();
        match resp {
            PreludeCreateVerificationResponse::Blocked { reason, .. } => {
                assert_eq!(reason, PreludeBlockedReason::Unknown("absent".to_string()));
            }
            other => panic!("Expected Blocked variant, got {:?}", other),
        }
    }

    #[test]
    fn deserialize_blocked_with_null_reason() {
        let json = r#"{"status":"blocked","id":"x","reason":null}"#;
        let resp: PreludeCreateVerificationResponse = serde_json::from_str(json).unwrap();
        match resp {
            PreludeCreateVerificationResponse::Blocked { reason, .. } => {
                assert_eq!(reason, PreludeBlockedReason::Unknown("absent".to_string()));
            }
            other => panic!("Expected Blocked variant, got {:?}", other),
        }
    }
}
