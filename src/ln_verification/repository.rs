use crate::{
    infrastructure::sql::UnifiedExecutor,
    ln_verification::{payment_hash::PaymentHash, verification_id::VerificationId},
};
use chrono::NaiveDateTime;
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LightningVerificationEntity {
    pub id: VerificationId,
    #[allow(dead_code)] // Used for internal Phoenix sync operations
    pub payment_hash: PaymentHash,
    pub amount_sat: i32,
    pub expires_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
    pub finalised_at: Option<NaiveDateTime>,
    pub signup_code: Option<String>,
}

impl LightningVerificationEntity {
    /// Check if the verification is finalised aka paid.
    pub fn is_finalised(&self) -> bool {
        self.finalised_at.is_some()
    }
}

/// Table name for Lightning verifications in the database
const TABLE_NAME: &str = "lightning_verifications";

#[derive(Clone, Debug)]
pub struct LnVerificationRepository;

impl LnVerificationRepository {
    /// Create a new Lightning verification record
    ///
    /// # Arguments
    /// * `payment_hash` - The payment hash of the Lightning invoice
    /// * `amount_sat` - The amount in satoshis of the Lightning invoice
    /// * `executor` - The executor to use to execute the query
    ///
    /// # Returns
    /// * `LightningVerificationEntity` - The created Lightning verification record
    ///
    /// # Errors
    /// * `sqlx::Error` - If the query fails
    pub async fn create_verification<'a>(
        payment_hash: &PaymentHash,
        amount_sat: u64,
        expires_at: NaiveDateTime,
        executor: &mut UnifiedExecutor<'a>,
    ) -> Result<LightningVerificationEntity, sqlx::Error> {
        let statement = Query::insert()
            .into_table(TABLE_NAME)
            .columns(["payment_hash", "amount_sat", "expires_at"])
            .values([
                Expr::value(payment_hash.as_str()),
                Expr::value(amount_sat),
                Expr::value(expires_at),
            ])
            .expect("Failed to build insert query")
            .returning_all()
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let con = executor.get_con().await?;
        let verification: LightningVerificationEntity =
            sqlx::query_as_with(&query, values).fetch_one(con).await?;

        Ok(verification)
    }

    /// Get a verification by its public ID (for API requests)
    ///
    /// # Arguments
    /// * `id` - The verification ID
    /// * `executor` - The executor to use to execute the query
    ///
    /// # Returns
    /// * `LightningVerificationEntity` - The verification record
    ///
    /// # Errors
    /// * `sqlx::Error` - If the query fails
    pub async fn get_verification_by_id<'a>(
        id: &VerificationId,
        executor: &mut UnifiedExecutor<'a>,
    ) -> Result<Option<LightningVerificationEntity>, sqlx::Error> {
        let statement = Query::select()
            .columns([
                "id",
                "payment_hash",
                "amount_sat",
                "created_at",
                "expires_at",
                "finalised_at",
                "signup_code",
            ])
            .from(TABLE_NAME)
            .and_where(Expr::col("id").eq(*id.as_uuid()))
            .to_owned();
        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let con = executor.get_con().await?;
        let verification: LightningVerificationEntity =
            match sqlx::query_as_with(&query, values).fetch_one(con).await {
                Ok(verification) => verification,
                Err(sqlx::Error::RowNotFound) => return Ok(None),
                Err(e) => return Err(e),
            };

        Ok(Some(verification))
    }

    /// Get a verification by payment hash
    ///
    /// # Arguments
    /// * `payment_hash` - The payment hash of the Lightning invoice
    /// * `executor` - The executor to use to execute the query
    ///
    /// # Returns
    /// * `LightningVerificationEntity` - The verification record
    ///
    /// # Errors
    /// * `sqlx::Error` - If the query fails
    pub async fn get_verification_by_payment_hash<'a>(
        payment_hash: &PaymentHash,
        executor: &mut UnifiedExecutor<'a>,
    ) -> Result<Option<LightningVerificationEntity>, sqlx::Error> {
        let statement = Query::select()
            .columns([
                "id",
                "payment_hash",
                "amount_sat",
                "created_at",
                "expires_at",
                "finalised_at",
                "signup_code",
            ])
            .from(TABLE_NAME)
            .and_where(Expr::col("payment_hash").eq(payment_hash.as_str()))
            .to_owned();
        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let con = executor.get_con().await?;
        let verification: LightningVerificationEntity =
            match sqlx::query_as_with(&query, values).fetch_one(con).await {
                Ok(verification) => verification,
                Err(sqlx::Error::RowNotFound) => return Ok(None),
                Err(e) => return Err(e),
            };

        Ok(Some(verification))
    }

    /// Update a verification to finalised
    ///
    /// # Arguments
    /// * `payment_hash` - The payment hash of the Lightning invoice
    /// * `signup_code` - The signup code to use for the verification
    /// * `executor` - The executor to use to execute the query
    ///
    /// # Returns
    /// * `LightningVerificationEntity` - The updated verification record
    ///
    /// # Errors
    /// * `sqlx::Error` - If the query fails
    pub async fn update_verification_finalised<'a>(
        payment_hash: &PaymentHash,
        signup_code: &str,
        executor: &mut UnifiedExecutor<'a>,
    ) -> Result<LightningVerificationEntity, sqlx::Error> {
        let statement = Query::update()
            .table(TABLE_NAME)
            .and_where(Expr::col("payment_hash").eq(payment_hash.as_str()))
            .values([
                ("finalised_at", Expr::current_timestamp().into()),
                ("signup_code", Some(signup_code.to_string()).into()),
            ])
            .returning_all()
            .to_owned();
        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let con = executor.get_con().await?;
        let verification: LightningVerificationEntity =
            sqlx::query_as_with(&query, values).fetch_one(con).await?;

        Ok(verification)
    }

    /// Get the last created at timestamp of the latest finalized verification
    ///
    /// # Arguments
    /// * `executor` - The executor to use to execute the query
    ///
    /// # Returns
    /// * `Option<NaiveDateTime>` - The last created at timestamp of the latest finalized verification. None if no verifications are finalized.
    ///
    /// # Errors
    /// * `sqlx::Error` - If the query fails
    pub async fn get_last_finalized_timestamp<'a>(
        executor: &mut UnifiedExecutor<'a>,
    ) -> Result<Option<NaiveDateTime>, sqlx::Error> {
        let statement = Query::select()
            .expr(Expr::col("created_at").max())
            .from(TABLE_NAME)
            .and_where(Expr::col("finalised_at").is_not_null())
            .group_by_col("created_at")
            .to_owned();
        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let con = executor.get_con().await?;
        let timestamp: Option<NaiveDateTime> = sqlx::query_scalar_with(&query, values)
            .fetch_optional(con)
            .await?;

        Ok(timestamp)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use sqlx::PgPool;

    use crate::infrastructure::sql::SqlDb;

    use super::*;

    #[sqlx::test]
    async fn test_create_get_verification(pool: PgPool) {
        let db = SqlDb::test(pool).await;
        let payment_hash = PaymentHash::random();
        let expires_at = Utc::now().naive_utc();
        let veri = LnVerificationRepository::create_verification(
            &payment_hash,
            1000,
            expires_at,
            &mut db.pool().into(),
        )
        .await
        .unwrap();
        assert_eq!(veri.payment_hash, payment_hash);
        assert_eq!(veri.amount_sat, 1000);
        assert!(veri.finalised_at.is_none());
        assert!(veri.signup_code.is_none());
        assert_eq!(
            veri.expires_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
            expires_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
        );

        // Same payment hash should fail
        LnVerificationRepository::create_verification(
            &payment_hash,
            1000,
            Utc::now().naive_utc(),
            &mut db.pool().into(),
        )
        .await
        .unwrap_err();

        let veri2 = LnVerificationRepository::get_verification_by_payment_hash(
            &payment_hash,
            &mut db.pool().into(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(veri2.payment_hash, payment_hash);
        assert_eq!(veri2.amount_sat, 1000);
        assert!(veri2.finalised_at.is_none());
        assert!(veri2.signup_code.is_none());
    }

    #[sqlx::test]
    async fn test_not_found(pool: PgPool) {
        let db = SqlDb::test(pool).await;

        let payment_hash = PaymentHash::random();
        let veri2 = LnVerificationRepository::get_verification_by_payment_hash(
            &payment_hash,
            &mut db.pool().into(),
        )
        .await
        .unwrap();
        assert!(veri2.is_none());
    }

    #[sqlx::test]
    async fn test_update_verification_finalised(pool: PgPool) {
        let db = SqlDb::test(pool).await;
        let payment_hash = PaymentHash::random();
        let veri = LnVerificationRepository::create_verification(
            &payment_hash,
            1000,
            Utc::now().naive_utc(),
            &mut db.pool().into(),
        )
        .await
        .unwrap();
        assert!(veri.finalised_at.is_none());
        assert!(veri.signup_code.is_none());

        let veri2 = LnVerificationRepository::update_verification_finalised(
            &veri.payment_hash,
            "123456",
            &mut db.pool().into(),
        )
        .await
        .unwrap();
        assert_eq!(veri2.payment_hash, payment_hash);
        assert_eq!(veri2.amount_sat, 1000);
        assert!(veri2.finalised_at.is_some());
        assert_eq!(veri2.signup_code, Some("123456".to_string()));
    }

    #[sqlx::test]
    async fn test_get_last_finalized_timestamp(pool: PgPool) {
        let db = SqlDb::test(pool).await;

        let timestamp_no_verifications =
            LnVerificationRepository::get_last_finalized_timestamp(&mut db.pool().into())
                .await
                .unwrap();
        assert!(timestamp_no_verifications.is_none());

        let payment_hash0 = PaymentHash::random();
        let _veri0 = LnVerificationRepository::create_verification(
            &payment_hash0,
            1000,
            Utc::now().naive_utc(),
            &mut db.pool().into(),
        )
        .await
        .unwrap();

        let payment_hash1 = PaymentHash::random();
        LnVerificationRepository::create_verification(
            &payment_hash1,
            1000,
            Utc::now().naive_utc(),
            &mut db.pool().into(),
        )
        .await
        .unwrap();
        let veri1 = LnVerificationRepository::update_verification_finalised(
            &payment_hash1,
            "1",
            &mut db.pool().into(),
        )
        .await
        .unwrap();

        let payment_hash2 = PaymentHash::random();
        let _veri2 = LnVerificationRepository::create_verification(
            &payment_hash2,
            1000,
            Utc::now().naive_utc(),
            &mut db.pool().into(),
        )
        .await
        .unwrap();

        let timestamp =
            LnVerificationRepository::get_last_finalized_timestamp(&mut db.pool().into())
                .await
                .unwrap();
        assert_eq!(timestamp, Some(veri1.created_at));
    }

    #[sqlx::test]
    async fn test_get_verification_by_id(pool: PgPool) {
        let db = SqlDb::test(pool).await;
        let payment_hash = PaymentHash::random();
        let expires_at = Utc::now().naive_utc();
        let veri = LnVerificationRepository::create_verification(
            &payment_hash,
            1000,
            expires_at,
            &mut db.pool().into(),
        )
        .await
        .unwrap();

        // Should be able to get by ID
        let veri_by_id =
            LnVerificationRepository::get_verification_by_id(&veri.id, &mut db.pool().into())
                .await
                .unwrap()
                .unwrap();

        assert_eq!(veri_by_id.id, veri.id);
        assert_eq!(veri_by_id.payment_hash, payment_hash);
        assert_eq!(veri_by_id.amount_sat, 1000);

        // Non-existent ID should return None
        let non_existent = LnVerificationRepository::get_verification_by_id(
            &VerificationId::random(),
            &mut db.pool().into(),
        )
        .await
        .unwrap();
        assert!(non_existent.is_none());
    }
}
