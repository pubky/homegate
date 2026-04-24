use std::net::IpAddr;

use crate::infrastructure::config::SignupQuotaConfig;
use crate::infrastructure::sql::{DbError, SqlDb, UnifiedExecutor};
use crate::shared::HomeserverAdminAPI;
use crate::sms_verification::HasherArgon2id;

use super::error::IpVerificationError;
use super::repository::IpVerificationRepository;
use super::types::IpVerificationResponse;

const WEEKLY_WINDOW_DAYS: i64 = 7;
const ANNUAL_WINDOW_DAYS: i64 = 365;

#[derive(Clone, Debug)]
pub struct IpVerificationService {
    db: SqlDb,
    homeserver_admin_api: HomeserverAdminAPI,
    hasher_argon2id: HasherArgon2id,
    max_verifications_per_week: u32,
    max_verifications_per_year: u32,
    signup_quota: Option<SignupQuotaConfig>,
}

impl IpVerificationService {
    pub fn new(
        db: SqlDb,
        homeserver_admin_api: HomeserverAdminAPI,
        max_verifications_per_week: u32,
        max_verifications_per_year: u32,
        signup_quota: Option<SignupQuotaConfig>,
    ) -> Self {
        Self {
            db,
            homeserver_admin_api,
            hasher_argon2id: HasherArgon2id::new(),
            max_verifications_per_week,
            max_verifications_per_year,
            signup_quota,
        }
    }

    pub async fn verify(
        &self,
        ip_address: IpAddr,
    ) -> Result<IpVerificationResponse, IpVerificationError> {
        let ip_hash = self.hasher_argon2id.hash(&ip_address.to_string());

        // Use a transaction with an advisory lock so the rate limit check and
        // insert are atomic, preventing concurrent requests from bypassing the
        // limit.
        let mut tx = self.db.pool().begin().await.map_err(DbError::from)?;
        self.acquire_advisory_lock(&mut tx, &ip_hash).await?;

        let mut executor: UnifiedExecutor<'_> = (&mut tx).into();
        self.check_rate_limits(&mut executor, &ip_hash).await?;
        drop(executor);

        let signup_code = self.generate_signup_token().await?;

        let mut executor: UnifiedExecutor<'_> = (&mut tx).into();
        IpVerificationRepository::create_verification(&mut executor, &ip_hash, &signup_code)
            .await?;

        drop(executor);
        tx.commit().await.map_err(DbError::from)?;

        Ok(IpVerificationResponse {
            signup_code,
            homeserver_pubky: self.homeserver_admin_api.get_homeserver_pubky(),
        })
    }

    /// Acquire a transaction-scoped advisory lock keyed on the IP hash.
    /// This serializes concurrent requests for the same IP while allowing
    /// different IPs to proceed in parallel. The lock is released
    /// automatically when the transaction ends.
    async fn acquire_advisory_lock(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        ip_hash: &str,
    ) -> Result<(), IpVerificationError> {
        let lock_key = advisory_lock_key(ip_hash);
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
        ip_hash: &str,
    ) -> Result<(), IpVerificationError> {
        let weekly_count = IpVerificationRepository::count_verifications_in_last_days(
            executor,
            ip_hash,
            WEEKLY_WINDOW_DAYS,
        )
        .await?;
        if weekly_count >= self.max_verifications_per_week as i64 {
            tracing::warn!(
                ip_hash = %ip_hash,
                weekly_count = weekly_count,
                weekly_limit = self.max_verifications_per_week,
                "Weekly IP verification limit exceeded"
            );
            return Err(IpVerificationError::WeeklyLimitExceeded);
        }

        let annual_count = IpVerificationRepository::count_verifications_in_last_days(
            executor,
            ip_hash,
            ANNUAL_WINDOW_DAYS,
        )
        .await?;
        if annual_count >= self.max_verifications_per_year as i64 {
            tracing::warn!(
                ip_hash = %ip_hash,
                annual_count = annual_count,
                annual_limit = self.max_verifications_per_year,
                "Annual IP verification limit exceeded"
            );
            return Err(IpVerificationError::AnnualLimitExceeded);
        }

        Ok(())
    }

    async fn generate_signup_token(&self) -> Result<String, IpVerificationError> {
        let result = match &self.signup_quota {
            Some(quota) => {
                self.homeserver_admin_api
                    .generate_signup_token_with_quota(quota)
                    .await
            }
            None => self.homeserver_admin_api.generate_signup_token().await,
        };
        result.map_err(|e| {
            tracing::error!(error = %e, "Failed to generate signup token");
            IpVerificationError::HomeserverUnavailable
        })
    }
}

/// Derive a stable i64 key for `pg_advisory_xact_lock` from an IP hash string.
fn advisory_lock_key(ip_hash: &str) -> i64 {
    let hash = blake3::hash(ip_hash.as_bytes());
    let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().expect("8 bytes");
    i64::from_le_bytes(bytes)
}
