use crate::HomeserverAdminAPI;
use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::phone_number::PhoneNumber;
use crate::sms_verification::prelude_api::{
    PreludeAPI, PreludeCheckCodeResponse, PreludeCreateVerificationResponse,
};
use crate::sms_verification::repository::SmsVerificationRepository;
use crate::sms_verification::types::{
    CreateVerificationRequest, CreateVerificationResponse, SendCodeRequest, SendCodeResponse,
};
use std::net::IpAddr;

#[derive(Clone, Debug)]
pub struct SmsVerificationService {
    repository: SmsVerificationRepository,
    prelude_api: PreludeAPI,
    homeserver_admin_api: HomeserverAdminAPI,
    max_verified_sessions: u32,
}

impl SmsVerificationService {
    pub fn new(
        repository: SmsVerificationRepository,
        prelude_api: PreludeAPI,
        homeserver_admin_api: HomeserverAdminAPI,
        max_verified_sessions: u32,
    ) -> Self {
        Self {
            repository,
            prelude_api,
            homeserver_admin_api,
            max_verified_sessions,
        }
    }

    /// Check if a phone number can create a new verification
    pub async fn check_verification_limit(
        &self,
        phone_number: &PhoneNumber,
    ) -> Result<(), SmsVerificationError> {
        let count = self
            .repository
            .count_verified_sessions(phone_number)
            .await
            .map_err(SmsVerificationError::DatabaseError)?;
        if count >= self.max_verified_sessions as i64 {
            return Err(SmsVerificationError::TooManyVerifiedSessions);
        }
        Ok(())
    }

    /// Initiates a phone number verification process
    pub async fn create_verification(
        &self,
        request: CreateVerificationRequest,
        ip_address: IpAddr,
    ) -> Result<CreateVerificationResponse, SmsVerificationError> {
        self.check_verification_limit(&request.phone_number).await?;

        let prelude_response = self
            .prelude_api
            .create_verification(request.phone_number.as_str(), Some(ip_address))
            .await?;

        let id = match &prelude_response {
            PreludeCreateVerificationResponse::Success { id } => id,
            PreludeCreateVerificationResponse::Retry { id } => {
                tracing::info!(
                    phone_number = %request.phone_number,
                    prelude_id = %id,
                    "User retrying verification code"
                );
                id
            }
            PreludeCreateVerificationResponse::Blocked { id, .. } => id,
        };

        self.repository
            .create_verification(&request.phone_number, id)
            .await?;

        if let PreludeCreateVerificationResponse::Blocked { id, reason } = &prelude_response {
            self.repository
                .mark_failed(id, &format!("{:?}", reason))
                .await?;
        }

        Ok(match prelude_response {
            PreludeCreateVerificationResponse::Success { .. } => {
                CreateVerificationResponse::Success
            }
            PreludeCreateVerificationResponse::Retry { .. } => CreateVerificationResponse::Retry,
            PreludeCreateVerificationResponse::Blocked { reason, .. } => {
                CreateVerificationResponse::Blocked { reason }
            }
        })
    }

    /// Validates a verification code for a phone number
    pub async fn send_code(
        &self,
        request: SendCodeRequest,
    ) -> Result<SendCodeResponse, SmsVerificationError> {
        // Confirm first that there's an active verification session in our database
        self.repository
            .check_pending_exists(&request.phone_number)
            .await
            .map_err(|_| {
                SmsVerificationError::NoActiveVerification(request.phone_number.to_string())
            })?;

        let prelude_response = self
            .prelude_api
            .check_code(request.phone_number.as_str(), &request.code)
            .await?;

        match prelude_response {
            PreludeCheckCodeResponse::Success { id, .. } => {
                let code = self.homeserver_admin_api.generate_signup_token().await?;
                self.repository.mark_verified(&id, &code).await?;
                Ok(SendCodeResponse::Success {
                    signup_code: code,
                    homeserver_pubky: self.homeserver_admin_api.get_homeserver_pubky(),
                })
            }
            PreludeCheckCodeResponse::ExpiredOrNotFound { id, .. } => {
                self.repository
                    .mark_failed(&id, "expired_or_not_found")
                    .await?;
                Ok(SendCodeResponse::ExpiredOrNotFound)
            }
            PreludeCheckCodeResponse::Failure { .. } => {
                // Wrong code - don't mark as failed, allow retries
                Ok(SendCodeResponse::Failure)
            }
        }
    }
}
