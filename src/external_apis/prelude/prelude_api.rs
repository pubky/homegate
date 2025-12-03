use crate::EnvConfig;
use crate::sms_verification::SmsVerificationError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct PreludeAPI {
    http_client: reqwest::Client,
    api_key: String,
    base_url: String,
}

#[derive(Serialize)]
struct VerificationRequest {
    target: Target,
    #[serde(skip_serializing_if = "Option::is_none")]
    signals: Option<Signals>,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct PreludeVerificationResponse {
    pub id: String,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Serialize)]
struct CheckCodeRequest {
    target: Target,
    code: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PreludeCheckCodeResponse {
    pub id: String,
    pub status: String,
    pub metadata: Option<serde_json::Value>,
    pub request_id: Option<String>,
}

impl PreludeAPI {
    pub fn from_config(config: &EnvConfig) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            api_key: config.prelude_api_key.clone(),
            base_url: config.prelude_api_url.clone(),
        }
    }
}

/// Trait for SMS verification provider API implementations
#[async_trait]
pub trait SmsVerificationProviderApi: Send + Sync {
    /// Creates a verification request for the given phone number
    async fn create_verification(
        &self,
        phone_number: &str,
        ip_address: Option<&str>,
    ) -> Result<PreludeVerificationResponse, SmsVerificationError>;

    /// Checks a verification code for the given phone number
    async fn check_code(
        &self,
        phone_number: &str,
        code: &str,
    ) -> Result<PreludeCheckCodeResponse, SmsVerificationError>;
}

#[async_trait]
impl SmsVerificationProviderApi for PreludeAPI {
    /// Creates a verification request for the given phone number
    async fn create_verification(
        &self,
        phone_number: &str,
        ip_address: Option<&str>,
    ) -> Result<PreludeVerificationResponse, SmsVerificationError> {
        let request_body = VerificationRequest {
            target: Target {
                target_type: "phone_number".to_string(),
                value: phone_number.to_string(),
            },
            signals: ip_address.map(|ip| Signals {
                ip_address: ip.to_string(),
            }),
        };

        let url = format!("{}/v2/verification", self.base_url);
        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            return Err(SmsVerificationError::ApiError {
                status: status.as_u16(),
                message: error_body,
            });
        }

        let verification_response = response
            .json::<PreludeVerificationResponse>()
            .await
            .map_err(|e| {
                SmsVerificationError::InvalidResponse(format!("Failed to parse response: {}", e))
            })?;

        Ok(verification_response)
    }

    /// Checks a verification code for the given phone number
    async fn check_code(
        &self,
        phone_number: &str,
        code: &str,
    ) -> Result<PreludeCheckCodeResponse, SmsVerificationError> {
        let request_body = CheckCodeRequest {
            target: Target {
                target_type: "phone_number".to_string(),
                value: phone_number.to_string(),
            },
            code: code.to_string(),
        };

        let url = format!("{}/v2/verification/check", self.base_url);
        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            return Err(SmsVerificationError::ApiError {
                status: status.as_u16(),
                message: error_body,
            });
        }

        let check_response = response
            .json::<PreludeCheckCodeResponse>()
            .await
            .map_err(|e| {
                SmsVerificationError::InvalidResponse(format!("Failed to parse response: {}", e))
            })?;

        Ok(check_response)
    }
}
