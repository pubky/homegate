#[cfg(test)]
use url::Url;

use crate::apple_verification::apple_id_token_verifier::{
    AppleIdTokenVerificationError, AppleJwksIdTokenVerifier,
};
use crate::infrastructure::config::AppleVerificationConfig;
use crate::infrastructure::sql::SqlDb;
use crate::shared::{HasherArgon2id, HomeserverAdminAPI};
use crate::shared::{
    LimitEnforcement, Metrics, RateLimitedSignupIssuer, SignupType, VerificationTable,
};

use super::error::AppleVerificationError;
use super::types::AppleVerificationResponse;

const APPLE_VERIFICATIONS_TABLE: VerificationTable = VerificationTable {
    name: "apple_verifications",
    hash_column: "apple_identity_hash",
    signup_type: SignupType::Apple,
};

#[derive(Clone, Debug)]
pub struct AppleVerificationService {
    signup_issuer: RateLimitedSignupIssuer,
    apple_id_token_verifier: AppleJwksIdTokenVerifier,
    hasher_argon2id: HasherArgon2id,
}

impl AppleVerificationService {
    pub fn new(
        db: SqlDb,
        homeserver_admin_api: HomeserverAdminAPI,
        config: &AppleVerificationConfig,
        hasher: HasherArgon2id,
        metrics: Metrics,
    ) -> Self {
        Self::from_verifier(
            db,
            homeserver_admin_api,
            config,
            hasher,
            metrics,
            AppleJwksIdTokenVerifier::new(config.allowed_audiences.clone()),
        )
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        db: SqlDb,
        homeserver_admin_api: HomeserverAdminAPI,
        config: &AppleVerificationConfig,
        hasher: HasherArgon2id,
        metrics: Metrics,
        jwks_url: Url,
    ) -> Self {
        Self::from_verifier(
            db,
            homeserver_admin_api,
            config,
            hasher,
            metrics,
            AppleJwksIdTokenVerifier::for_test(config.allowed_audiences.clone(), jwks_url),
        )
    }

    fn from_verifier(
        db: SqlDb,
        homeserver_admin_api: HomeserverAdminAPI,
        config: &AppleVerificationConfig,
        hasher: HasherArgon2id,
        metrics: Metrics,
        apple_id_token_verifier: AppleJwksIdTokenVerifier,
    ) -> Self {
        Self {
            signup_issuer: RateLimitedSignupIssuer::new(
                db,
                homeserver_admin_api,
                APPLE_VERIFICATIONS_TABLE,
                config.max_verifications_per_week,
                config.max_verifications_per_year,
                None,
                metrics,
            ),
            apple_id_token_verifier,
            hasher_argon2id: hasher,
        }
    }

    pub async fn verify(
        &self,
        apple_id_token: &str,
    ) -> Result<AppleVerificationResponse, AppleVerificationError> {
        let identity = self
            .apple_id_token_verifier
            .verify(apple_id_token)
            .await
            .map_err(|error| match error {
                AppleIdTokenVerificationError::Invalid => {
                    AppleVerificationError::InvalidAppleIdToken
                }
                AppleIdTokenVerificationError::DependencyUnavailable => {
                    AppleVerificationError::AppleVerifierUnavailable
                }
            })?;
        let apple_identity_hash = self
            .hasher_argon2id
            .hash(&format!("{}\n{}", identity.issuer, identity.subject))
            .await?;

        let issued = self
            .signup_issuer
            .issue(&apple_identity_hash, LimitEnforcement::Enforce)
            .await?;

        Ok(AppleVerificationResponse {
            signup_code: issued.signup_code,
            homeserver_pubky: issued.homeserver_pubky,
        })
    }
}
