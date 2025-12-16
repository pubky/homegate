use crate::infrastructure::database::unified_executor::UnifiedExecutor;
use chrono::NaiveDateTime;
use sea_query::{Expr, PostgresQueryBuilder, Query};
use sea_query_binder::SqlxBinder;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LightningVerificationEntity {
    pub id: i32,
    pub payment_hash: String,
    pub amount_sat: i32,
    pub created_at: NaiveDateTime,
    pub finalised_at: Option<NaiveDateTime>,
    pub signup_code: Option<String>,
}

impl LightningVerificationEntity {
    /// Check if the verification is finalised aka paid.
    pub fn is_finalised(&self) -> bool {
        self.finalised_at.is_some()
    }

    /// This is the first 8 characters of the payment hash.
    /// This is used to identify the payment towards the user in case of customer support requests.
    pub fn payment_reference(&self) -> String {
        self.payment_hash[..8].to_string()
    }
}

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
        payment_hash: &str,
        amount_sat: u64,
        executor: &mut UnifiedExecutor<'a>,
    ) -> Result<LightningVerificationEntity, sqlx::Error> {
        let statement = Query::insert()
            .into_table("lightning_verifications")
            .columns(["payment_hash", "amount_sat"])
            .values([Expr::value(payment_hash), Expr::value(amount_sat)])
            .expect("Failed to build insert query")
            .returning_all()
            .to_owned();

        let (query, values) = statement.build_sqlx(PostgresQueryBuilder);
        let con = executor.get_con().await?;
        let verification: LightningVerificationEntity =
            sqlx::query_as_with(&query, values).fetch_one(con).await?;

        Ok(verification)
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
        payment_hash: &str,
        executor: &mut UnifiedExecutor<'a>,
    ) -> Result<Option<LightningVerificationEntity>, sqlx::Error> {
        let statement = Query::select()
            .columns([
                "id",
                "payment_hash",
                "amount_sat",
                "created_at",
                "finalised_at",
                "signup_code",
            ])
            .from("lightning_verifications")
            .and_where(Expr::col("payment_hash").eq(payment_hash))
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
        payment_hash: &str,
        signup_code: &str,
        executor: &mut UnifiedExecutor<'a>,
    ) -> Result<LightningVerificationEntity, sqlx::Error> {
        let statement = Query::update()
            .table("lightning_verifications")
            .and_where(Expr::col("payment_hash").eq(payment_hash))
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
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use crate::SqlDb;

    use super::*;

    #[sqlx::test]
    async fn test_create_get_verification(pool: PgPool) {
        let db = SqlDb::test(pool).await;
        let veri =
            LnVerificationRepository::create_verification("12345678", 1000, &mut db.pool().into())
                .await
                .unwrap();
        assert_eq!(veri.payment_hash, "12345678");
        assert_eq!(veri.amount_sat, 1000);
        assert!(veri.finalised_at.is_none());
        assert!(veri.signup_code.is_none());

        let veri2 = LnVerificationRepository::get_verification_by_payment_hash(
            "12345678",
            &mut db.pool().into(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(veri2.payment_hash, "12345678");
        assert_eq!(veri2.amount_sat, 1000);
        assert!(veri2.finalised_at.is_none());
        assert!(veri2.signup_code.is_none());
    }

    #[sqlx::test]
    async fn test_not_found(pool: PgPool) {
        let db = SqlDb::test(pool).await;

        let veri2 = LnVerificationRepository::get_verification_by_payment_hash(
            "12345678",
            &mut db.pool().into(),
        )
        .await
        .unwrap();
        assert!(veri2.is_none());
    }

    #[sqlx::test]
    async fn test_update_verification_finalised(pool: PgPool) {
        let db = SqlDb::test(pool).await;
        let veri =
            LnVerificationRepository::create_verification("12345678", 1000, &mut db.pool().into())
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
        assert_eq!(veri2.payment_hash, "12345678");
        assert_eq!(veri2.amount_sat, 1000);
        assert!(veri2.finalised_at.is_some());
        assert_eq!(veri2.signup_code, Some("123456".to_string()));
    }
}
