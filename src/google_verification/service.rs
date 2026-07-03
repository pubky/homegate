use std::sync::Arc;

use crate::google_verification::google_id_token_verifier::{
    GoogleIdTokenVerificationError, GoogleIdTokenVerifier, GoogleJwksIdTokenVerifier,
};
use crate::infrastructure::config::GoogleVerificationConfig;
use crate::infrastructure::sql::{DbError, SqlDb, UnifiedExecutor};
use crate::shared::{HasherArgon2id, HomeserverAdminAPI};

use super::error::GoogleVerificationError;
use super::repository::GoogleVerificationRepository;
use super::types::GoogleVerificationResponse;

const WEEKLY_WINDOW_DAYS: i64 = 7;
const ANNUAL_WINDOW_DAYS: i64 = 365;

#[derive(Clone, Debug)]
pub struct GoogleVerificationService {
    db: SqlDb,
    homeserver_admin_api: HomeserverAdminAPI,
    google_id_token_verifier: Arc<dyn GoogleIdTokenVerifier>,
    hasher_argon2id: HasherArgon2id,
    max_verifications_per_week: u32,
    max_verifications_per_year: u32,
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
            db,
            homeserver_admin_api,
            google_id_token_verifier,
            hasher_argon2id: hasher,
            max_verifications_per_week: config.max_verifications_per_week,
            max_verifications_per_year: config.max_verifications_per_year,
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

        let mut tx = self.db.pool().begin().await.map_err(DbError::from)?;
        self.acquire_advisory_lock(&mut tx, &google_identity_hash)
            .await?;

        let mut executor: UnifiedExecutor<'_> = (&mut tx).into();
        self.check_rate_limits(&mut executor, &google_identity_hash)
            .await?;
        drop(executor);

        let signup_code = self.generate_signup_token().await?;

        let mut executor: UnifiedExecutor<'_> = (&mut tx).into();
        GoogleVerificationRepository::create_verification(
            &mut executor,
            &google_identity_hash,
            &signup_code,
        )
        .await?;

        drop(executor);
        tx.commit().await.map_err(DbError::from)?;

        Ok(GoogleVerificationResponse {
            signup_code,
            homeserver_pubky: self.homeserver_admin_api.get_homeserver_pubky(),
        })
    }

    async fn acquire_advisory_lock(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        google_identity_hash: &str,
    ) -> Result<(), GoogleVerificationError> {
        let lock_key = advisory_lock_key(google_identity_hash);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut **tx)
            .await
            .map_err(DbError::from)?;
        Ok(())
    }

    async fn check_rate_limits(
        &self,
        executor: &mut UnifiedExecutor<'_>,
        google_identity_hash: &str,
    ) -> Result<(), GoogleVerificationError> {
        let weekly_count = GoogleVerificationRepository::count_verifications_in_last_days(
            executor,
            google_identity_hash,
            WEEKLY_WINDOW_DAYS,
        )
        .await?;
        if weekly_count >= self.max_verifications_per_week as i64 {
            tracing::warn!(
                weekly_count = weekly_count,
                weekly_limit = self.max_verifications_per_week,
                "Weekly Google verification limit exceeded"
            );
            return Err(GoogleVerificationError::WeeklyLimitExceeded);
        }

        let annual_count = GoogleVerificationRepository::count_verifications_in_last_days(
            executor,
            google_identity_hash,
            ANNUAL_WINDOW_DAYS,
        )
        .await?;
        if annual_count >= self.max_verifications_per_year as i64 {
            tracing::warn!(
                annual_count = annual_count,
                annual_limit = self.max_verifications_per_year,
                "Annual Google verification limit exceeded"
            );
            return Err(GoogleVerificationError::AnnualLimitExceeded);
        }

        Ok(())
    }

    async fn generate_signup_token(&self) -> Result<String, GoogleVerificationError> {
        self.homeserver_admin_api
            .generate_signup_token()
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "Failed to generate signup token");
                GoogleVerificationError::HomeserverUnavailable
            })
    }
}

fn advisory_lock_key(google_identity_hash: &str) -> i64 {
    let hash = blake3::hash(google_identity_hash.as_bytes());
    let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().expect("8 bytes");
    i64::from_le_bytes(bytes)
}
