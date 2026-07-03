use std::sync::Arc;

use crate::google_verification::google_id_token_verifier::{
    GoogleIdTokenVerificationError, GoogleIdTokenVerifier, GoogleJwksIdTokenVerifier,
};
use crate::infrastructure::config::GoogleVerificationConfig;
use crate::infrastructure::sql::SqlDb;
use crate::shared::{HasherArgon2id, HomeserverAdminAPI};
use crate::shared::{LimitEnforcement, RateLimitedSignupIssuer, VerificationTable};

use super::error::GoogleVerificationError;
use super::types::GoogleVerificationResponse;

const GOOGLE_VERIFICATIONS_TABLE: VerificationTable = VerificationTable {
    name: "google_verifications",
    hash_column: "google_identity_hash",
};

#[derive(Clone, Debug)]
pub struct GoogleVerificationService {
    signup_issuer: RateLimitedSignupIssuer,
    google_id_token_verifier: Arc<dyn GoogleIdTokenVerifier>,
    hasher_argon2id: HasherArgon2id,
}

impl GoogleVerificationService {
    pub fn new(
        db: SqlDb,
        homeserver_admin_api: HomeserverAdminAPI,
        config: &GoogleVerificationConfig,
        hasher: HasherArgon2id,
    ) -> Self {
        Self::with_verifier(
            db,
            homeserver_admin_api,
            config,
            hasher,
            Arc::new(GoogleJwksIdTokenVerifier::new(
                config.google_client_id.clone(),
            )),
        )
    }

    pub(crate) fn with_verifier(
        db: SqlDb,
        homeserver_admin_api: HomeserverAdminAPI,
        config: &GoogleVerificationConfig,
        hasher: HasherArgon2id,
        google_id_token_verifier: Arc<dyn GoogleIdTokenVerifier>,
    ) -> Self {
        Self {
            signup_issuer: RateLimitedSignupIssuer::new(
                db,
                homeserver_admin_api,
                GOOGLE_VERIFICATIONS_TABLE,
                config.max_verifications_per_week,
                config.max_verifications_per_year,
            ),
            google_id_token_verifier,
            hasher_argon2id: hasher,
        }
    }

    pub async fn verify(
        &self,
        google_id_token: &str,
    ) -> Result<GoogleVerificationResponse, GoogleVerificationError> {
        let identity = self
            .google_id_token_verifier
            .verify(google_id_token)
            .await
            .map_err(|error| match error {
                GoogleIdTokenVerificationError::Invalid => {
                    GoogleVerificationError::InvalidGoogleIdToken
                }
                GoogleIdTokenVerificationError::DependencyUnavailable => {
                    GoogleVerificationError::GoogleVerifierUnavailable
                }
            })?;
        let google_identity_hash = self
            .hasher_argon2id
            .hash(&format!("{}\n{}", identity.issuer, identity.subject));

        let issued = self
            .signup_issuer
            .issue(&google_identity_hash, LimitEnforcement::Enforce, None)
            .await?;

        Ok(GoogleVerificationResponse {
            signup_code: issued.signup_code,
            homeserver_pubky: issued.homeserver_pubky,
        })
    }
}
