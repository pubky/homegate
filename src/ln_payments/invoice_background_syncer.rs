use chrono::{Days, Utc};
use url::Url;

use crate::{SqlDb, ln_payments::{LightningVerificationEntity, LnVerificationRepository, phoenixd_api::{GetIncomingInvoiceResponse, PhoenixdAPI}}};

/// Struct that syncronizes the lightning payments with the database.
pub struct InvoiceBackgroundSyncer {
    db: SqlDb,
    phoenixd_api: PhoenixdAPI,
}

impl InvoiceBackgroundSyncer {
    pub fn new(db: SqlDb, phoenixd_api_url: &Url, phoenixd_api_password: &str) -> Self {
        let phoenixd_api = PhoenixdAPI::new(phoenixd_api_url, phoenixd_api_password);
        Self { db, phoenixd_api }
    }

    /// Catch up on all paid invoices from the last 14 days
    /// This is done in case the server was offline for a while and we need to catch up on all paid invoices.
    async fn catchup_paid_invoices(&self) -> Result<(), anyhow::Error> {
        let from = Utc::now()
            .checked_sub_days(Days::new(14))
            .expect("14 days ago is always valid");
        let limit = 100; // Pull up to 100 invoices at a time
        let mut offset = 0;
        loop {
            let invoices = self
                .phoenixd_api
                .list_paid_invoices(from, Some(limit), Some(offset), false)
                .await?;

            for invoice in invoices.iter() {
                self.sync_invoice(invoice).await?;
            }
            let no_more_invoices = invoices.is_empty();
            if no_more_invoices {
                break;
            }
            offset += limit;
        };
        Ok(())
    }

    /// Sync one invoice with the database
    async fn sync_invoice(&self, invoice: &GetIncomingInvoiceResponse) -> Result<(), anyhow::Error> {
        let verification = match LnVerificationRepository::get_verification_by_payment_hash(&invoice.payment_hash, &mut self.db.pool().into()).await? {
            Some(verification) => verification,
            None => {
                // Not a invoice of ours, skip it
                return Ok(());
            }
        };
        if verification.is_finalised() {
            // Already finalised, skip it
            return Ok(());
        }
        self.finalise_invoice(&verification).await?;
        Ok(())
    }

    /// Finalise an invoice
    async fn finalise_invoice(&self, invoice: &LightningVerificationEntity) -> Result<(), anyhow::Error> {
        let verification = LnVerificationRepository::get_verification_by_payment_hash(&invoice.payment_hash, &mut self.db.pool().into()).await?;
        if verification.is_none() {
            return Err(anyhow::anyhow!("Verification not found"));
        }
        let verification = verification.unwrap();
        LnVerificationRepository::update_verification_finalised(&invoice.payment_hash, &invoice.signup_code, &mut self.db.pool().into()).await?;
        Ok(())
    }

    pub async fn run(&self) -> Result<(), anyhow::Error> {
        let invoices = self
            .phoenixd_api
            .list_paid_invoices(Utc::now(), None, None, false)
            .await?;
        Ok(())
    }
}
