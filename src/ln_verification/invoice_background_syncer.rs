use std::time::Duration;

use crate::ln_verification::{
    LightningVerificationEntity, error::LnVerificationError, payment_hash::PaymentHash,
    phoenixd_api::PhoenixdAPI, service::LnVerificationService,
};

use futures_util::TryStreamExt;
use tokio::sync::broadcast;

/// Struct that syncronizes the PhoenixD payments with the database.
#[derive(Clone, Debug)]
pub struct InvoiceBackgroundSyncer {
    service: LnVerificationService,
    phoenixd_api: PhoenixdAPI,
    verification_completed_tx: broadcast::Sender<PaymentHash>,
}

impl InvoiceBackgroundSyncer {
    pub async fn new(service: LnVerificationService, phoenixd_api: PhoenixdAPI) -> Self {
        let (tx, _) = broadcast::channel(200);

        Self {
            service,
            phoenixd_api,
            verification_completed_tx: tx,
        }
    }

    /// Subscribe to finalized verification events.
    /// Returns a receiver that will receive notifications whenever a verification is finalized.
    pub fn subscribe(&self) -> broadcast::Receiver<PaymentHash> {
        self.verification_completed_tx.subscribe()
    }

    /// Await the payment to be finalized.
    /// Returns Ok(Some(LightningVerificationEntity)) if the payment was finalized, Ok(None) if the timeout was reached.
    pub async fn wait_for_payment(
        &self,
        payment_hash: &PaymentHash,
        timeout: Duration,
    ) -> Result<Option<LightningVerificationEntity>, LnVerificationError> {
        let future = async {
            let mut receiver = self.subscribe();
            loop {
                let finalized_payment_hash = receiver.recv().await.expect("Should never happen");
                if finalized_payment_hash == *payment_hash {
                    break;
                }
            }
        };

        match tokio::time::timeout(timeout, future).await {
            Ok(_) => {}
            Err(_) => return Ok(None),
        };

        self.service.get_verification(payment_hash).await
    }

    /// Sync an invoice.
    /// If the invoice is finalized, it will be sent to the verification completed channel.
    /// # Errors
    /// * `LnVerificationError` - If the invoice sync fails
    async fn sync_invoice(&self, payment_hash: &PaymentHash) -> Result<(), LnVerificationError> {
        let newly_finalised = match self.service.sync_invoice(payment_hash).await? {
            Some(verification) => verification,
            None => return Ok(()),
        };
        tracing::info!("Verification {} finalised", newly_finalised.payment_hash);
        let _ = self
            .verification_completed_tx
            .send(newly_finalised.payment_hash);
        Ok(())
    }

    /// Catch up on all paid invoices
    /// This is done in case the server was offline for a while and we need to catch up on all paid invoices.
    async fn catchup_paid_invoices(&self) -> Result<(), LnVerificationError> {
        let from = self.service.get_catchup_start_timestamp().await?;
        let limit = 100; // Pull up to 100 invoices at a time
        let mut offset = 0;
        loop {
            let invoices = self
                .phoenixd_api
                .list_paid_invoices(from.and_utc(), Some(limit), Some(offset), false)
                .await?;

            if invoices.is_empty() {
                break;
            }
            for invoice in invoices.iter() {
                self.sync_invoice(&invoice.payment_hash).await?;
            }

            offset += limit;
        }
        Ok(())
    }

    /// Run the invoice background syncer.
    /// This will connect to the PhoenixD websocket and listen for payments.
    /// It will also catch up on any paid invoices that we might have missed.
    /// # Errors
    /// * `LnVerificationError` - If the websocket connection fails or the invoice sync fails
    pub async fn run(&self) -> Result<(), LnVerificationError> {
        let mut websocket = self.phoenixd_api.received_payments_websocket().await?;
        tracing::info!("Websocket connected");
        tracing::info!("Catching up on paid invoices that we might have missed...");
        self.catchup_paid_invoices().await?;
        tracing::info!("Listening for live payments...");
        loop {
            let event = match websocket
                .try_next()
                .await
                .map_err(LnVerificationError::Phoenixd)?
            {
                Some(event) => event,
                None => break, // websocket closed
            };
            self.sync_invoice(&event.payment_hash).await?;
        }
        tracing::warn!("Websocket closed");
        Ok(())
    }
}
