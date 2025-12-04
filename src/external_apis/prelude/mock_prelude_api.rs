use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::external_apis::prelude::prelude_api::{
    PreludeCheckCodeResponse, PreludeVerificationResponse, SmsVerificationProviderApi,
};
use crate::sms_verification::SmsVerificationError;

/// Mock SMS verification provider API for testing which attempts to mimic Prelude's v2 api's behaviour.
///
/// Always uses code "123456" for all verifications
#[derive(Clone)]
pub struct MockSmsVerificationProviderApi {
    // Maps phone_number -> (verification_id, code)
    verifications: Arc<Mutex<HashMap<String, (String, String)>>>,
}

impl Default for MockSmsVerificationProviderApi {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSmsVerificationProviderApi {
    pub fn new() -> Self {
        Self {
            verifications: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl SmsVerificationProviderApi for MockSmsVerificationProviderApi {
    async fn create_verification(
        &self,
        phone_number: &str,
        _ip_address: Option<&str>,
    ) -> Result<PreludeVerificationResponse, SmsVerificationError> {
        let mut verifications = self.verifications.lock().unwrap();

        // Check if verification already exists for this phone number
        if let Some((existing_id, _)) = verifications.get(phone_number) {
            // Return retry status with existing verification ID
            return Ok(PreludeVerificationResponse {
                id: existing_id.clone(),
                status: "retry".to_string(),
                reason: Some("Verification already in progress".to_string()),
            });
        }

        // Create new verification
        let verification_id = Uuid::new_v4().to_string();
        let code = "123456".to_string();

        verifications.insert(phone_number.to_string(), (verification_id.clone(), code));

        Ok(PreludeVerificationResponse {
            id: verification_id,
            status: "success".to_string(),
            reason: None,
        })
    }

    async fn check_code(
        &self,
        phone_number: &str,
        code: &str,
    ) -> Result<PreludeCheckCodeResponse, SmsVerificationError> {
        let mut verifications = self.verifications.lock().unwrap();

        if let Some((verification_id, stored_code)) = verifications.get(phone_number) {
            let status = if code == stored_code {
                "success"
            } else {
                "failure"
            };

            let response = PreludeCheckCodeResponse {
                id: verification_id.clone(),
                status: status.to_string(),
                metadata: None,
                request_id: None,
            };

            // Remove verification from state after successful check
            // This allows a new send_code to create a fresh verification
            if status == "success" {
                verifications.remove(phone_number);
            }

            Ok(response)
        } else {
            Err(SmsVerificationError::InvalidResponse(
                "No verification found for phone number".to_string(),
            ))
        }
    }
}
