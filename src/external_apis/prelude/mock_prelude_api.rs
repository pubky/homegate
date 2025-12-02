#[cfg(test)]
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::external_apis::prelude::prelude_api::{
    CheckCodeResponse, SmsVerificationProviderApi, VerificationResponse,
};
use crate::sms_verification::SmsVerificationError;

/// Mock SMS verification provider API for testing
///
/// Always uses code "123456" for all verifications
/// Stores state in-memory for verification lookups
#[derive(Clone)]
pub struct MockSmsVerificationProviderApi {
    // Maps phone_number -> (verification_id, code)
    verifications: Arc<Mutex<HashMap<String, (String, String)>>>,
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
    ) -> Result<VerificationResponse, SmsVerificationError> {
        let verification_id = Uuid::new_v4().to_string();
        let code = "123456".to_string();

        self.verifications
            .lock()
            .unwrap()
            .insert(phone_number.to_string(), (verification_id.clone(), code));

        Ok(VerificationResponse {
            id: verification_id,
            status: "success".to_string(),
            reason: None,
        })
    }

    async fn check_code(
        &self,
        phone_number: &str,
        code: &str,
    ) -> Result<CheckCodeResponse, SmsVerificationError> {
        let verifications = self.verifications.lock().unwrap();

        if let Some((verification_id, stored_code)) = verifications.get(phone_number) {
            let status = if code == stored_code {
                "success"
            } else {
                "failed"
            };

            Ok(CheckCodeResponse {
                id: verification_id.clone(),
                status: status.to_string(),
                metadata: None,
                request_id: None,
            })
        } else {
            Err(SmsVerificationError::InvalidResponse(
                "No verification found for phone number".to_string(),
            ))
        }
    }
}
