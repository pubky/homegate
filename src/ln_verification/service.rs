use std::str::FromStr;

use chrono::{NaiveDateTime, TimeDelta};

use crate::{infrastructure::sql::{SqlDb, UnifiedExecutor}, ln_verification::{LightningVerificationEntity, LnVerificationRepository, error::LnVerificationError, payment_hash::PaymentHash, phoenixd_api::PhoenixdAPI}, shared::HomeserverAdminAPI};

#[derive(Clone, Debug)]
pub struct LnVerificationService {
    db: SqlDb,
    phoenix_api: PhoenixdAPI,
    homeserver_api: HomeserverAdminAPI, 
    amount_sat: u64,
    invoice_description: String,
    invoice_expiry_seconds: u64,
}

impl LnVerificationService {
    pub fn new(db: SqlDb, phoenix_api: PhoenixdAPI, homeserver_api: HomeserverAdminAPI, amount_sat: u64, invoice_description: String, invoice_expiry_seconds: u64) -> Self {
        Self { db, phoenix_api, homeserver_api, amount_sat, invoice_description, invoice_expiry_seconds  }
    }

    /// Create a new verification including a Lightning invoice.
    /// Returns the verification and the invoice.
    /// # Errors
    /// * `LnVerificationError` - If the verification or invoice creation fails
    pub async fn create_verification(&self) -> Result<(LightningVerificationEntity, super::phoenixd_api::GetIncomingInvoiceResponse), LnVerificationError> {
        let invoice = self.phoenix_api.create_invoice(self.amount_sat, &self.invoice_description, self.invoice_expiry_seconds).await?;
        let invoice: super::phoenixd_api::GetIncomingInvoiceResponse = self.phoenix_api.get_invoice(&invoice.payment_hash).await?;
        let verification = LnVerificationRepository::create_verification(&invoice.payment_hash, invoice.requested_sat, &mut self.db.pool().into()).await?;
        Ok((verification, invoice))
    }

    /// Sync an phonenixd invoice with the database.
    /// This is necessary to keep our database in sync with the phoenixd invoice status.
    /// Returns the updated verification if the invoice was finalized, None if the invoice was not found or already finalised.
    pub async fn sync_invoice(&self, payment_hash: &PaymentHash) -> Result<Option<LightningVerificationEntity>, LnVerificationError> {
        let mut tx = self.db.pool().begin().await?;

        let verification = match LnVerificationRepository::get_verification_by_payment_hash(payment_hash, &mut UnifiedExecutor::from_tx(&mut tx)).await? {
            Some(verification) => verification,
            None => {
                return Ok(None);
            }
        };
        if verification.is_finalised() {
            return Ok(None);
        }

        let invoice = self.phoenix_api.get_invoice(payment_hash).await?;
        if !invoice.is_paid {
            return Ok(None);
        }

        let signup_code = self.homeserver_api.generate_signup_token().await.map_err(LnVerificationError::Homeserver)?;
        let verification = LnVerificationRepository::update_verification_finalised(payment_hash, &signup_code, &mut UnifiedExecutor::from_tx(&mut tx)).await?;
        tx.commit().await?;

        Ok(Some(verification))
    }

    /// Get the start timestamp for the catchup.
    /// This is the the created at timestamp of the last finalized verification, minus the invoice expiry time.
    /// If not invoice in the database is available, the default start date is returned.
    pub async fn get_catchup_start_timestamp(&self) -> Result<NaiveDateTime, LnVerificationError> {
        let timestamp = LnVerificationRepository::get_last_finalized_timestamp(&mut self.db.pool().into()).await?;
        let timestamp = timestamp.map(|timestamp| timestamp.checked_sub_signed(TimeDelta::seconds(self.invoice_expiry_seconds as i64)).expect("Infalliable"));
        let default_start_date = NaiveDateTime::from_str("2025-12-01T00:00:00").expect("Infalliable");
        let timestamp = timestamp.unwrap_or(default_start_date);
        Ok(timestamp)
    }


    /// Get a verification by its payment hash.
    /// Returns the verification if it exists, None if it does not exist.
    /// # Errors
    /// * `LnVerificationError` - If the verification retrieval fails
    pub async fn get_verification(&self, payment_hash: &PaymentHash) -> Result<Option<LightningVerificationEntity>, LnVerificationError> {
        let verification = LnVerificationRepository::get_verification_by_payment_hash(payment_hash, &mut self.db.pool().into()).await?;
        Ok(verification)
    }
}