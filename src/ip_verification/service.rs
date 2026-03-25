use std::net::IpAddr;

use crate::infrastructure::sql::{DbError, SqlDb, UnifiedExecutor};
use crate::shared::HomeserverAdminAPI;
use crate::sms_verification::HasherArgon2id;

use super::error::IpVerificationError;
use super::repository::IpVerificationRepository;
use super::types::IpVerificationResponse;

#[derive(Clone, Debug)]
pub struct IpVerificationService {
    homeserver_admin_api: HomeserverAdminAPI,
    hasher_argon2id: HasherArgon2id,
    max_verifications_per_week: u32,
    max_verifications_per_year: u32,
    enabled: bool,
}

impl IpVerificationService {
    pub fn new(
        homeserver_admin_api: HomeserverAdminAPI,
        max_verifications_per_week: u32,
        max_verifications_per_year: u32,
        enabled: bool,
    ) -> Self {
        Self {
            homeserver_admin_api,
            hasher_argon2id: HasherArgon2id::new(),
            max_verifications_per_week,
            max_verifications_per_year,
            enabled,
        }
    }

    pub async fn verify(
        &self,
        db: &SqlDb,
        ip_address: IpAddr,
    ) -> Result<IpVerificationResponse, IpVerificationError> {
        if !self.enabled {
            return Err(IpVerificationError::ServiceDisabled);
        }

        let ip_hash = self
            .hasher_argon2id
            .hash_phone_number(&ip_address.to_string());

        // Use a transaction with an advisory lock so the rate limit check and
        // insert are atomic, preventing concurrent requests from bypassing the
        // limit.
        let mut tx = db.pool().begin().await.map_err(DbError::from)?;

        // Acquire a transaction-scoped advisory lock keyed on the IP hash.
        // This serializes concurrent requests for the same IP while allowing
        // different IPs to proceed in parallel. The lock is released
        // automatically when the transaction ends.
        let lock_key = advisory_lock_key(&ip_hash);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;

        let mut executor: UnifiedExecutor<'_> = (&mut tx).into();

        // Check weekly limit
        let weekly_count =
            IpVerificationRepository::count_verifications_in_last_days(&mut executor, &ip_hash, 7)
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

        // Check annual limit
        let annual_count = IpVerificationRepository::count_verifications_in_last_days(
            &mut executor,
            &ip_hash,
            365,
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

        // Drop the executor to release the borrow on tx before the network
        // call. The advisory lock is still held on the transaction.
        drop(executor);

        // Generate signup token only after rate limits pass
        let signup_code = self
            .homeserver_admin_api
            .generate_signup_token()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to generate signup token");
                IpVerificationError::HomeserverUnavailable
            })?;

        // Record the verification
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
}

/// Derive a stable i64 key for `pg_advisory_xact_lock` from an IP hash string.
fn advisory_lock_key(ip_hash: &str) -> i64 {
    let hash = blake3::hash(ip_hash.as_bytes());
    let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().expect("8 bytes");
    i64::from_le_bytes(bytes)
}
