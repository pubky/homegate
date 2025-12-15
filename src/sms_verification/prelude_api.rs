use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use url::Url;

use crate::sms_verification::{Code, PhoneNumber};

#[derive(Debug)]
pub enum PreludeError {
    RateLimited { retry_after: Option<u64> },
    RequestFailed(reqwest::Error),
}

impl PreludeError {
    /// Check if response is a 429, extract retry-after header if present and return PreludeApi error.
    /// Otherwise, pass back the same reqwest::Response
    pub fn from_response(response: reqwest::Response) -> Result<reqwest::Response, Self> {
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            return Err(PreludeError::RateLimited { retry_after });
        }
        Ok(response)
    }
}

impl std::fmt::Display for PreludeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreludeError::RateLimited { .. } => write!(f, "Rate limit exceeded"),
            PreludeError::RequestFailed(e) => write!(f, "Request failed: {}", e),
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
    ip_address: String,
}
#[derive(Serialize)]
struct PreludeCreateVerificationRequest {
    target: Target,
    #[serde(skip_serializing_if = "Option::is_none")]
    signals: Option<Signals>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreludeBlockedReason {
    /// The signature of the SDK signals is expired. They should be sent within the hour following their collection.
    ExpiredSignature,
    /// The phone number is part of the configured block list.
    InBlockList,
    /// The phone number is not a valid line number (e.g. landline).
    InvalidPhoneLine,
    /// The signature of the SDK signals is invalid.
    InvalidSignature,
    /// The phone number has made too many verification attempts.
    RepeatedAttempts,
    /// The verification attempt was deemed suspicious by the anti-fraud system.
    Suspicious,
    /// Prelude API returned Blocked status without a reason.
    Unknown,
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
        ip_address: Option<IpAddr>,
    ) -> Result<PreludeCreateVerificationResponse, PreludeError> {
        let request_body = PreludeCreateVerificationRequest {
            target: Target {
                target_type: "phone_number".to_string(),
                value: phone_number.to_string(),
            },
            signals: ip_address.map(|ip| Signals {
                ip_address: ip.to_string(),
            }),
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

        let response = PreludeError::from_response(response)?;
        let verification_response = response
            .error_for_status()?
            .json::<PreludeCreateVerificationResponse>()
            .await?;

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
        let response = PreludeError::from_response(response)?;
        let check_response = response
            .error_for_status()?
            .json::<PreludeCheckCodeResponse>()
            .await?;

        Ok(check_response)
    }
}
