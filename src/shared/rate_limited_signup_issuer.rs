//! Shared "verify identity → issue signup code" core used by verification providers.
//!
//! Every low-friction verification route (currently `ip_verification` and
//! `google_verification`) ends the same way: once a provider-specific identity
//! has been verified and reduced to a peppered hash, homegate must atomically
//! enforce weekly/annual issuance limits for that identity and hand out a
//! homeserver signup code. This module owns that final step so providers don't
//! re-implement it.
//!
//! # Adding a new verification provider
//!
//! A new provider slice (e.g. `apple_verification`) only needs to implement
//! what is genuinely provider-specific and delegate the rest to
//! [`RateLimitedSignupIssuer`]:
//!
//! 1. **Verify the identity** however the provider requires (e.g. validate an
//!    ID token against the provider's keys). Keep this behind a small trait so
//!    tests can fake it — see `google_verification::google_id_token_verifier`.
//! 2. **Derive a stable identity hash** with [`crate::shared::HasherArgon2id`],
//!    e.g. `hasher.hash(&format!("{issuer}\n{subject}"))`. Never store or log
//!    raw identifiers.
//! 3. **Create a table** to record issuance, via a migration in
//!    `infrastructure::sql::migrations` with the columns `id`, `<your>_hash`,
//!    `signup_code`, `created_at`, plus an index on `(<your>_hash, created_at)`
//!    — copy `m20260630_create_google_verifications`.
//! 4. **Describe the table** with a [`VerificationTable`] const in your service
//!    and call [`RateLimitedSignupIssuer::issue`] with the identity hash.
//! 5. Keep the HTTP layer, request/response DTOs, error enum, and config
//!    section per-slice (see `google_verification` for the pattern), mapping
//!    [`SignupIssuanceError`] into your error enum with a `From` impl.
//!
//! Rate-limit windows are fixed at 7/365 days; the per-window maximums come
//! from the provider's config section.

use chrono::NaiveDateTime;
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;
use sqlx::Row;

use crate::infrastructure::config::SignupQuotaConfig;
use crate::infrastructure::sql::{DbError, SqlDb, UnifiedExecutor};
use crate::shared::HomeserverAdminAPI;

const WEEKLY_WINDOW_DAYS: i64 = 7;
const ANNUAL_WINDOW_DAYS: i64 = 365;

/// The table a provider records issued verifications in.
#[derive(Clone, Copy, Debug)]
pub struct VerificationTable {
    /// Table name, e.g. `"google_verifications"`.
    pub name: &'static str,
    /// Column holding the peppered identity hash, e.g. `"google_identity_hash"`.
    pub hash_column: &'static str,
}

/// Whether rate limits apply to a given issuance.
///
/// [`LimitEnforcement::Bypass`] exists for whitelist escape hatches (e.g. the
/// `ip_verification` `limit_whitelist`); the issuance is still recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitEnforcement {
    Enforce,
    Bypass,
}

#[derive(thiserror::Error, Debug)]
pub enum SignupIssuanceError {
    #[error("weekly limit exceeded")]
    WeeklyLimitExceeded,

    #[error("annual limit exceeded")]
    AnnualLimitExceeded,

    #[error("homeserver unavailable")]
    HomeserverUnavailable,

    #[error(transparent)]
    Database(#[from] DbError),
}

/// A successfully issued signup code.
#[derive(Clone, Debug)]
pub struct IssuedSignup {
    pub signup_code: String,
    pub homeserver_pubky: String,
}

/// Atomically enforces per-identity issuance limits and issues homeserver
/// signup codes, recording each issuance in the provider's table.
#[derive(Clone, Debug)]
pub struct RateLimitedSignupIssuer {
    db: SqlDb,
    homeserver_admin_api: HomeserverAdminAPI,
    table: VerificationTable,
    max_verifications_per_week: u32,
    max_verifications_per_year: u32,
}

impl RateLimitedSignupIssuer {
    pub fn new(
        db: SqlDb,
        homeserver_admin_api: HomeserverAdminAPI,
        table: VerificationTable,
        max_verifications_per_week: u32,
        max_verifications_per_year: u32,
    ) -> Self {
        Self {
            db,
            homeserver_admin_api,
            table,
            max_verifications_per_week,
            max_verifications_per_year,
        }
    }

    /// Issue a signup code for a verified identity.
    ///
    /// Runs in a transaction holding a per-identity advisory lock so the rate
    /// limit check and insert are atomic — concurrent requests for the same
    /// identity cannot bypass the limit.
    ///
    /// `signup_quota` applies quota limits to the issued code via the
    /// homeserver POST endpoint. This is an `ip_verification`-only concept
    /// (signing up "low tier" users); other providers should pass `None`,
    /// which issues codes with homeserver system defaults (GET).
    pub async fn issue(
        &self,
        identity_hash: &str,
        enforcement: LimitEnforcement,
        signup_quota: Option<&SignupQuotaConfig>,
    ) -> Result<IssuedSignup, SignupIssuanceError> {
        let mut tx = self.db.pool().begin().await.map_err(DbError::from)?;
        self.acquire_advisory_lock(&mut tx, identity_hash).await?;

        let mut executor: UnifiedExecutor<'_> = (&mut tx).into();
        if enforcement == LimitEnforcement::Enforce {
            self.check_rate_limits(&mut executor, identity_hash).await?;
        }
        drop(executor);

        // The homeserver HTTP call happens while holding the advisory lock for
        // this identity. Keeping the call inside the transaction means we never
        // record a verification without a valid signup code.
        let signup_code = self.generate_signup_token(signup_quota).await?;

        let mut executor: UnifiedExecutor<'_> = (&mut tx).into();
        self.record_verification(&mut executor, identity_hash, &signup_code)
            .await?;

        drop(executor);
        tx.commit().await.map_err(DbError::from)?;

        Ok(IssuedSignup {
            signup_code,
            homeserver_pubky: self.homeserver_admin_api.get_homeserver_pubky(),
        })
    }

    /// Acquire a transaction-scoped advisory lock keyed on the identity hash.
    /// This serializes concurrent requests for the same identity while allowing
    /// different identities to proceed in parallel. The lock is released
    /// automatically when the transaction ends.
    async fn acquire_advisory_lock(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        identity_hash: &str,
    ) -> Result<(), SignupIssuanceError> {
        let lock_key = advisory_lock_key(identity_hash);
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
        identity_hash: &str,
    ) -> Result<(), SignupIssuanceError> {
        let weekly_count = self
            .count_verifications_in_last_days(executor, identity_hash, WEEKLY_WINDOW_DAYS)
            .await?;
        if weekly_count >= self.max_verifications_per_week as i64 {
            tracing::warn!(
                table = self.table.name,
                identity_hash = %identity_hash,
                weekly_count = weekly_count,
                weekly_limit = self.max_verifications_per_week,
                "Weekly verification limit exceeded"
            );
            return Err(SignupIssuanceError::WeeklyLimitExceeded);
        }

        let annual_count = self
            .count_verifications_in_last_days(executor, identity_hash, ANNUAL_WINDOW_DAYS)
            .await?;
        if annual_count >= self.max_verifications_per_year as i64 {
            tracing::warn!(
                table = self.table.name,
                identity_hash = %identity_hash,
                annual_count = annual_count,
                annual_limit = self.max_verifications_per_year,
                "Annual verification limit exceeded"
            );
            return Err(SignupIssuanceError::AnnualLimitExceeded);
        }

        Ok(())
    }

    async fn generate_signup_token(
        &self,
        signup_quota: Option<&SignupQuotaConfig>,
    ) -> Result<String, SignupIssuanceError> {
        let result = match signup_quota {
            Some(quota) => {
                self.homeserver_admin_api
                    .generate_signup_token_with_quota(quota)
                    .await
            }
            None => self.homeserver_admin_api.generate_signup_token().await,
        };
        result.map_err(|error| {
            tracing::error!(error = %error, "Failed to generate signup token");
            SignupIssuanceError::HomeserverUnavailable
        })
    }

    /// Count verifications for an identity hash in the last N days.
    async fn count_verifications_in_last_days(
        &self,
        executor: &mut UnifiedExecutor<'_>,
        identity_hash: &str,
        days: i64,
    ) -> Result<i64, DbError> {
        let since = chrono::Utc::now().naive_utc() - chrono::Duration::days(days);
        self.count_verifications_since(executor, identity_hash, since)
            .await
    }

    /// Count verifications for an identity hash since a given timestamp.
    async fn count_verifications_since(
        &self,
        executor: &mut UnifiedExecutor<'_>,
        identity_hash: &str,
        since: NaiveDateTime,
    ) -> Result<i64, DbError> {
        let statement = Query::select()
            .expr(Expr::col("id").count())
            .from(self.table.name)
            .and_where(Expr::col(self.table.hash_column).eq(identity_hash))
            .and_where(Expr::col("created_at").gte(since))
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let row = sqlx::query_with(&query, values)
            .fetch_one(executor.get_con().await?)
            .await
            .map_err(DbError::from)?;
        let count: i64 = row.try_get(0).map_err(DbError::from)?;
        Ok(count)
    }

    /// Insert a new verification record.
    async fn record_verification(
        &self,
        executor: &mut UnifiedExecutor<'_>,
        identity_hash: &str,
        signup_code: &str,
    ) -> Result<(), DbError> {
        let statement = Query::insert()
            .into_table(self.table.name)
            .columns([self.table.hash_column, "signup_code"])
            .values([identity_hash.into(), signup_code.into()])
            .expect("Failed to build insert query")
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        sqlx::query_with(&query, values)
            .execute(executor.get_con().await?)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }
}

/// Derive a stable i64 key for `pg_advisory_xact_lock` from an identity hash.
fn advisory_lock_key(identity_hash: &str) -> i64 {
    let hash = blake3::hash(identity_hash.as_bytes());
    let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().expect("8 bytes");
    i64::from_le_bytes(bytes)
}
