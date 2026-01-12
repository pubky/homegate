use crate::infrastructure::sql::{SqlDb, UnifiedExecutor};
use crate::shared::HomeserverAdminAPI;
use crate::sms_verification::HasherArgon2id;
use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::prelude_api::{
    PreludeAPI, PreludeCheckCodeResponse, PreludeCreateVerificationResponse,
};
use crate::sms_verification::repository::SmsVerificationRepository;
use crate::sms_verification::types::{
    CreateVerificationRequest, ValidateCodeRequest, ValidateCodeResponse,
};
use std::net::IpAddr;

#[derive(Clone, Debug)]
pub struct SmsVerificationService {
    prelude_api: PreludeAPI,
    homeserver_admin_api: HomeserverAdminAPI,
    hasher_argon2id: HasherArgon2id,
    max_verifications_per_week: u32,
    max_verifications_per_year: u32,
    limit_whitelist: Vec<String>,
}

impl SmsVerificationService {
    pub fn new(
        prelude_api: PreludeAPI,
        homeserver_admin_api: HomeserverAdminAPI,
        max_verifications_per_week: u32,
        max_verifications_per_year: u32,
        limit_whitelist: Vec<String>,
    ) -> Self {
        Self {
            prelude_api,
            homeserver_admin_api,
            hasher_argon2id: HasherArgon2id::new(),
            max_verifications_per_week,
            max_verifications_per_year,
            limit_whitelist,
        }
    }

    /// Check if a phone number has reached its limits for new verificaitons
    pub async fn check_verification_limit(
        &mut self,
        executor: &mut UnifiedExecutor<'_>,
        phone_number: &str,
        phone_number_hash: &str,
    ) -> Result<(), SmsVerificationError> {
        if self.limit_whitelist.iter().any(|w| w == phone_number) {
            return Ok(());
        }
        let weekly_count = SmsVerificationRepository::count_verified_sessions_in_last_days(
            executor,
            phone_number_hash,
            7,
        )
        .await?;
        if weekly_count >= self.max_verifications_per_week as i64 {
            tracing::warn!(
                phone_number = %phone_number_hash,
                weekly_count = weekly_count,
                weekly_limit = self.max_verifications_per_week,
                "Weekly verification limit exceeded"
            );
            return Err(SmsVerificationError::WeeklyLimitExceeded);
        }

        let annual_count = SmsVerificationRepository::count_verified_sessions_in_last_days(
            executor,
            phone_number_hash,
            365,
        )
        .await?;
        if annual_count >= self.max_verifications_per_year as i64 {
            tracing::warn!(
                phone_number = %phone_number_hash,
                annual_count = annual_count,
                annual_limit = self.max_verifications_per_year,
                "Annual verification limit exceeded"
            );
            return Err(SmsVerificationError::AnnualLimitExceeded);
        }
        Ok(())
    }

    /// Initiates a phone number verification process
    pub async fn create_verification(
        &mut self,
        db: &SqlDb,
        request: CreateVerificationRequest,
        ip_address: IpAddr,
        user_agent: Option<String>,
    ) -> Result<(), SmsVerificationError> {
        let phone_number_hash = self
            .hasher_argon2id
            .hash_phone_number(request.phone_number.as_str());

        let mut executor: UnifiedExecutor<'_> = db.pool().into();

        self.check_verification_limit(
            &mut executor,
            request.phone_number.as_str(),
            &phone_number_hash,
        )
        .await?;

        let prelude_response = self
            .prelude_api
            .create_verification(
                &request.phone_number,
                ip_address,
                user_agent,
                request.dispatch_id,
            )
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

        SmsVerificationRepository::create_verification(&mut executor, &phone_number_hash, id)
            .await?;

        if let PreludeCreateVerificationResponse::Blocked { id, reason } = &prelude_response {
            tracing::info!(
                "Phone number blocked for reason: {:?}. prelude id: {}",
                reason,
                id
            );
            SmsVerificationRepository::mark_failed(&mut executor, id, &format!("{:?}", reason))
                .await?;

            return Err(SmsVerificationError::Blocked);
        }

        // Return Ok for success or retry
        Ok(())
    }

    /// Validates a verification code for a phone number
    pub async fn validate_code(
        &mut self,
        db: &SqlDb,
        request: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, SmsVerificationError> {
        let phone_number_hash = self
            .hasher_argon2id
            .hash_phone_number(request.phone_number.as_str());

        let mut executor: UnifiedExecutor<'_> = db.pool().into();
        SmsVerificationRepository::err_if_no_active_verification(&mut executor, &phone_number_hash)
            .await
            .map_err(|_| SmsVerificationError::NoActiveVerification)?;

        let prelude_response = self
            .prelude_api
            .check_code(&request.phone_number, &request.code)
            .await?;

        match prelude_response {
            PreludeCheckCodeResponse::Success { id, .. } => {
                let code = match self.homeserver_admin_api.generate_signup_token().await {
                    Ok(code) => code,
                    Err(_e) => {
                        if let Err(e) = SmsVerificationRepository::mark_failed(
                            &mut executor,
                            &id,
                            "homeserver_signup_token_generation_failed",
                        )
                        .await
                        {
                            tracing::error!("{}", e);
                        }
                        return Err(SmsVerificationError::HomeserverUnavailable);
                    }
                };

                SmsVerificationRepository::mark_verified(&mut executor, &id, &code).await?;

                Ok(ValidateCodeResponse::Valid {
                    signup_code: code,
                    homeserver_pubky: self.homeserver_admin_api.get_homeserver_pubky(),
                })
            }
            PreludeCheckCodeResponse::Failure { .. } => {
                // Wrong code - don't mark as failed, allow retries
                Ok(ValidateCodeResponse::Invalid)
            }
            PreludeCheckCodeResponse::ExpiredOrNotFound { .. } => {
                // Do not return errors - user gets NoActiveVerification error
                if let Err(e) = SmsVerificationRepository::mark_all_pending_verification_as_failed(
                    &mut executor,
                    &phone_number_hash,
                    "expired_or_not_found",
                )
                .await
                {
                    tracing::error!("{}", e);
                }
                Err(SmsVerificationError::NoActiveVerification)
            }
        }
    }
}
