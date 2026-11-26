use std::str::FromStr;
use std::time::Duration;

use chrono::{NaiveDateTime, TimeDelta};
use futures_util::TryStreamExt;
use tokio::sync::broadcast;

use crate::{
    infrastructure::sql::{SqlDb, UnifiedExecutor},
    ln_verification::{
        LightningVerificationEntity, LnVerificationRepository, VerificationId,
        error::LnVerificationError, payment_hash::PaymentHash, phoenixd_api::PhoenixdAPI,
        types::VerificationResponse,
    },
    shared::{Dependency, HomeserverAdminAPI, Metrics, SignupType},
};

/// Service for the Lightning Network verification.
#[derive(Clone, Debug)]
pub struct LnVerificationService {
    db: SqlDb,
    phoenix_api: PhoenixdAPI,
    homeserver_api: HomeserverAdminAPI,
    completion_tx: broadcast::Sender<VerificationId>,
    amount_sat: u64,
    invoice_description: String,
    invoice_expiry_seconds: u64,
    metrics: Metrics,
}

impl LnVerificationService {
    pub fn new(
        db: SqlDb,
        phoenix_api: PhoenixdAPI,
        homeserver_api: HomeserverAdminAPI,
        amount_sat: u64,
        invoice_description: String,
        invoice_expiry_seconds: u64,
        metrics: Metrics,
    ) -> Self {
        let (tx, _) = broadcast::channel(200);
        Self {
            db,
            phoenix_api,
            homeserver_api,
            amount_sat,
            invoice_description,
            invoice_expiry_seconds,
            completion_tx: tx,
            metrics,
        }
    }

    /// Create a new verification including a Lightning invoice.
    /// Returns the verification and the invoice.
    /// # Errors
    /// * `LnVerificationError` - If the verification or invoice creation fails
    pub async fn create_verification(
        &self,
    ) -> Result<
        (
            LightningVerificationEntity,
            super::phoenixd_api::GetIncomingInvoiceResponse,
        ),
        LnVerificationError,
    > {
        let invoice = self
            .phoenix_api
            .create_invoice(
                self.amount_sat,
                &self.invoice_description,
                self.invoice_expiry_seconds,
            )
            .await?;
        let invoice: super::phoenixd_api::GetIncomingInvoiceResponse =
            self.phoenix_api.get_invoice(&invoice.payment_hash).await?;
        let expires_at = invoice.expires_at.naive_utc();
        let verification = LnVerificationRepository::create_verification(
            &invoice.payment_hash,
            invoice.requested_sat,
            expires_at,
            &mut self.db.pool().into(),
        )
        .await?;
        self.metrics
            .record_verification_started(SignupType::Lightning);
        Ok((verification, invoice))
    }

    /// Sync a phoenixd invoice with the database, issuing a signup code once the invoice is paid.
    /// This is necessary to keep our database in sync with the phoenixd invoice status.
    /// The verification row is locked before the signup code is requested, so concurrent
    /// syncs of one paid invoice run one after the other and only the first requests a code.
    /// # Returns
    /// * `Ok(Some(verification))` - The verification as it is after this sync, finalised or not
    /// * `Ok(None)` - If there is no verification for this payment hash
    /// # Errors
    /// * `LnVerificationError` - If database, phoenixd or homeserver errors occur
    async fn sync_invoice(
        &self,
        payment_hash: &PaymentHash,
    ) -> Result<Option<LightningVerificationEntity>, LnVerificationError> {
        // Ask phoenixd before taking the row lock, so that polls of an unpaid invoice do not
        // queue behind each other's phoenixd calls. Paid is final, so this read cannot go stale.
        let invoice = self.phoenix_api.get_invoice(payment_hash).await?;

        let mut tx = self.db.pool().begin().await?;

        let verification = match LnVerificationRepository::lock_verification_by_payment_hash(
            payment_hash,
            &mut UnifiedExecutor::from_tx(&mut tx),
        )
        .await?
        {
            Some(verification) => verification,
            None => {
                return Ok(None);
            }
        };
        if verification.is_finalised() || !invoice.is_paid {
            return Ok(Some(verification));
        }

        let signup_code = match self.homeserver_api.generate_signup_token().await {
            Ok(code) => code,
            Err(e) => {
                tracing::warn!(
                    payment_hash = %payment_hash.as_str(),
                    error = %e,
                    "Homeserver unavailable during invoice sync"
                );
                return Err(LnVerificationError::HomeserverUnavailable);
            }
        };
        let verification = LnVerificationRepository::update_verification_finalised(
            payment_hash,
            &signup_code,
            &mut UnifiedExecutor::from_tx(&mut tx),
        )
        .await?;
        tx.commit().await?;
        self.metrics
            .record_signup_code_issued(SignupType::Lightning);

        tracing::info!("Verification {} finalised", verification.id);
        if let Err(e) = self.completion_tx.send(verification.id.clone()) {
            tracing::warn!(
                "Failed to broadcast verification completion for {}: {}",
                verification.id,
                e
            );
        }

        Ok(Some(verification))
    }

    /// Get the start timestamp for the catchup.
    /// This is the the created at timestamp of the last finalized verification, minus the invoice expiry time.
    /// If not invoice in the database is available, the default start date is returned.
    async fn get_catchup_start_timestamp(&self) -> Result<NaiveDateTime, LnVerificationError> {
        let timestamp =
            LnVerificationRepository::get_last_finalized_timestamp(&mut self.db.pool().into())
                .await?;
        let timestamp = timestamp.map(|timestamp| {
            timestamp
                .checked_sub_signed(TimeDelta::seconds(self.invoice_expiry_seconds as i64))
                .expect("Infallible")
        });
        let default_start_date =
            NaiveDateTime::from_str("2025-12-01T00:00:00").expect("Infallible");
        let timestamp = timestamp.unwrap_or(default_start_date);
        Ok(timestamp)
    }

    /// Get a verification by its verification ID.
    /// Returns the verification if it exists, None if it does not exist.
    /// # Errors
    /// * `LnVerificationError` - If the verification retrieval fails
    async fn get_verification(
        &self,
        id: &VerificationId,
    ) -> Result<Option<LightningVerificationEntity>, LnVerificationError> {
        let verification =
            LnVerificationRepository::get_verification_by_id(id, &mut self.db.pool().into())
                .await?;
        Ok(verification)
    }

    /// Get the configured price for Lightning invoices in satoshis.
    pub fn get_price_sat(&self) -> u64 {
        self.amount_sat
    }

    /// Get a verification and sync it with PhoenixD if not finalized.
    /// This ensures we have the latest payment state, catching cases where:
    /// - PhoenixD webhooks were missed
    /// - Homeserver was down during initial sync
    ///
    /// Returns None if verification doesn't exist.
    pub async fn get_and_sync_verification(
        &self,
        id: &VerificationId,
    ) -> Result<Option<LightningVerificationEntity>, LnVerificationError> {
        let verification = match self.get_verification(id).await? {
            Some(v) => v,
            None => return Ok(None),
        };

        if verification.is_finalised() {
            return Ok(Some(verification));
        }

        self.sync_invoice(&verification.payment_hash).await
    }

    /// Get a verification and wait for it to finalize if needed.
    /// - Subscribes to broadcast channel FIRST to avoid race condition
    /// - Then gets and syncs the verification
    /// - If not finalized, waits up to `timeout` for payment using pre-subscribed receiver
    ///
    /// # Returns
    /// * `Ok(Some(VerificationResponse::Success))` - Verification finalized (already or within timeout)
    /// * `Ok(Some(VerificationResponse::TimedOut))` - Verification exists but payment not received within timeout
    /// * `Ok(None)` - Verification doesn't exist
    /// * `Err(_)` - Database or API error
    pub async fn get_and_await_verification(
        &self,
        id: &VerificationId,
        timeout: Duration,
    ) -> Result<Option<VerificationResponse>, LnVerificationError> {
        let receiver = self.subscribe();

        let verification = match self.get_and_sync_verification(id).await? {
            Some(v) => v,
            None => return Ok(None),
        };

        if verification.is_finalised() {
            // Already finalized
            return Ok(Some(VerificationResponse::Success(verification)));
        }

        match self
            .wait_for_payment_with_receiver(id, timeout, receiver)
            .await?
        {
            Some(v) => Ok(Some(VerificationResponse::Success(v))),
            None => Ok(Some(VerificationResponse::TimedOut)),
        }
    }

    /// Subscribe to verification completion events.
    /// Returns a receiver that will receive notifications whenever a verification is finalized.
    fn subscribe(&self) -> broadcast::Receiver<VerificationId> {
        self.completion_tx.subscribe()
    }

    /// Wait for a verification to be finalized using a pre-subscribed receiver.
    /// Returns Ok(Some(verification)) if finalized within timeout, Ok(None) if timeout exceeded.
    async fn wait_for_payment_with_receiver(
        &self,
        id: &VerificationId,
        timeout: Duration,
        mut receiver: broadcast::Receiver<VerificationId>,
    ) -> Result<Option<LightningVerificationEntity>, LnVerificationError> {
        let future = async {
            loop {
                match receiver.recv().await {
                    Ok(finalized_id) if finalized_id == *id => break,
                    Ok(_) => continue, // Different verification ID, keep waiting
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Broadcast receiver lagged by {} messages", n);
                        // Check if our verification was in the missed messages
                        if let Ok(Some(v)) = self.get_verification(id).await
                            && v.is_finalised()
                        {
                            break;
                        }
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::error!("Broadcast channel closed unexpectedly");
                        return;
                    }
                }
            }
        };

        match tokio::time::timeout(timeout, future).await {
            Ok(_) => {}
            Err(_) => return Ok(None),
        };

        self.get_verification(id).await
    }

    /// Catch up on all paid invoices.
    /// This is done in case the server was offline for a while.
    async fn catchup_paid_invoices(&self) -> Result<(), LnVerificationError> {
        let from = self.get_catchup_start_timestamp().await?;
        let limit = 100; // Pull up to 100 invoices at a time
        let mut offset = 0;
        loop {
            let invoices = self
                .phoenix_api
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

    /// Run the background invoice syncer.
    /// This connects to the PhoenixD websocket and listens for payment events.
    /// It also catches up on any paid invoices that might have been missed.
    ///
    /// This method should be spawned in a background task:
    /// ```
    /// tokio::spawn(async move {
    ///     if let Err(e) = service.run_background_sync().await {
    ///         tracing::error!(error = %e, "Background sync failed");
    ///     }
    /// });
    /// ```
    pub async fn run_background_sync(&self) -> Result<(), LnVerificationError> {
        let mut websocket = self.phoenix_api.received_payments_websocket().await?;
        tracing::info!("Websocket connected");
        tracing::info!("Catching up on paid invoices that we might have missed...");
        if let Err(e) = self.catchup_paid_invoices().await {
            match e {
                // Continue gracefully on Homeserver error, will retry on next API call or sync
                LnVerificationError::HomeserverUnavailable => {}
                e => return Err(e),
            }
        }
        tracing::info!("Listening for live payments...");
        loop {
            let event = match websocket
                .try_next()
                .await
                .inspect_err(|_| self.metrics.record_dependency_error(Dependency::Phoenixd))
                .map_err(LnVerificationError::Phoenixd)?
            {
                Some(event) => event,
                None => break, // websocket closed
            };
            if let Err(e) = self.sync_invoice(&event.payment_hash).await {
                match e {
                    // Continue gracefully on Homeserver error, will retry on next API call or sync
                    LnVerificationError::HomeserverUnavailable => {}
                    e => return Err(e),
                }
            }
        }
        tracing::warn!("Websocket closed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::{
        WiremockServers, setup_homeserver_signup_token,
        setup_homeserver_slow_distinct_signup_tokens, start_phoenixd_mock,
    };
    use crate::shared::Dependency;
    use sqlx::PgPool;
    use wiremock::MockServer;

    fn service(
        db: SqlDb,
        phoenixd: &MockServer,
        servers: &WiremockServers,
        metrics: Metrics,
    ) -> LnVerificationService {
        LnVerificationService::new(
            db,
            PhoenixdAPI::new(&phoenixd.uri().parse().unwrap(), "unused", metrics.clone()),
            HomeserverAdminAPI::new(
                &servers.homeserver_server.uri().parse().unwrap(),
                "test-pass",
                "test-homeserver-pubky",
                metrics.clone(),
            ),
            1000,
            "Test Invoice".to_string(),
            600,
            metrics,
        )
    }

    #[sqlx::test]
    async fn metrics_follow_a_paid_verification(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let phoenixd = start_phoenixd_mock(true).await;
        let metrics = Metrics::new();
        let service = service(
            SqlDb::test(pool).await,
            &phoenixd,
            &servers,
            metrics.clone(),
        );
        setup_homeserver_signup_token("token-123")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let (verification, _invoice) = service.create_verification().await.unwrap();
        assert_eq!(metrics.verification_started(SignupType::Lightning), 1);
        assert_eq!(metrics.signup_codes_issued(SignupType::Lightning), 0);

        let synced = service
            .get_and_sync_verification(&verification.id)
            .await
            .unwrap()
            .unwrap();
        assert!(synced.is_finalised());
        assert_eq!(metrics.signup_codes_issued(SignupType::Lightning), 1);
        assert_eq!(metrics.dependency_errors(Dependency::Homeserver), 0);
    }

    async fn stored_signup_code(
        service: &LnVerificationService,
        id: &VerificationId,
    ) -> Option<String> {
        service
            .get_verification(id)
            .await
            .unwrap()
            .unwrap()
            .signup_code
    }

    #[sqlx::test]
    async fn concurrent_verification_requests_share_one_signup_code(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let phoenixd = start_phoenixd_mock(true).await;
        let metrics = Metrics::new();
        let service = service(
            SqlDb::test(pool).await,
            &phoenixd,
            &servers,
            metrics.clone(),
        );
        setup_homeserver_slow_distinct_signup_tokens()
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;
        let (verification, _invoice) = service.create_verification().await.unwrap();

        let (first, second) = tokio::join!(
            service.get_and_sync_verification(&verification.id),
            service.get_and_sync_verification(&verification.id),
        );

        let first = first.unwrap().unwrap().signup_code;
        let second = second.unwrap().unwrap().signup_code;
        assert!(first.is_some());
        assert_eq!(second, first);
        assert_eq!(stored_signup_code(&service, &verification.id).await, first);
        assert_eq!(metrics.signup_codes_issued(SignupType::Lightning), 1);
    }

    #[sqlx::test]
    async fn background_sync_and_verification_request_share_one_signup_code(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let phoenixd = start_phoenixd_mock(true).await;
        let metrics = Metrics::new();
        let service = service(
            SqlDb::test(pool).await,
            &phoenixd,
            &servers,
            metrics.clone(),
        );
        setup_homeserver_slow_distinct_signup_tokens()
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;
        let (verification, _invoice) = service.create_verification().await.unwrap();

        // The background syncer calls `sync_invoice` for every payment event.
        let (background, request) = tokio::join!(
            service.sync_invoice(&verification.payment_hash),
            service.get_and_sync_verification(&verification.id),
        );

        let background = background.unwrap().unwrap().signup_code;
        let request = request.unwrap().unwrap().signup_code;
        assert!(background.is_some());
        assert_eq!(request, background);
        assert_eq!(
            stored_signup_code(&service, &verification.id).await,
            background
        );
        assert_eq!(metrics.signup_codes_issued(SignupType::Lightning), 1);
    }

    #[sqlx::test]
    async fn metrics_count_homeserver_failure_during_sync(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let phoenixd = start_phoenixd_mock(true).await;
        let metrics = Metrics::new();
        let service = service(
            SqlDb::test(pool).await,
            &phoenixd,
            &servers,
            metrics.clone(),
        );
        // No homeserver mock: token generation fails.

        let (verification, _invoice) = service.create_verification().await.unwrap();
        let result = service.get_and_sync_verification(&verification.id).await;

        assert!(matches!(
            result,
            Err(LnVerificationError::HomeserverUnavailable)
        ));
        assert_eq!(metrics.dependency_errors(Dependency::Homeserver), 1);
        assert_eq!(metrics.signup_codes_issued(SignupType::Lightning), 0);
    }

    #[sqlx::test]
    async fn metrics_do_not_count_unpaid_invoices(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let phoenixd = start_phoenixd_mock(false).await;
        let metrics = Metrics::new();
        let service = service(
            SqlDb::test(pool).await,
            &phoenixd,
            &servers,
            metrics.clone(),
        );

        let (verification, _invoice) = service.create_verification().await.unwrap();
        let synced = service
            .get_and_sync_verification(&verification.id)
            .await
            .unwrap()
            .unwrap();

        assert!(!synced.is_finalised());
        assert_eq!(metrics.verification_started(SignupType::Lightning), 1);
        assert_eq!(metrics.signup_codes_issued(SignupType::Lightning), 0);
    }

    #[sqlx::test]
    async fn metrics_count_phoenixd_failures(pool: PgPool) {
        let servers = WiremockServers::start().await;
        // No routes mounted: invoice creation fails.
        let phoenixd = MockServer::start().await;
        let metrics = Metrics::new();
        let service = service(
            SqlDb::test(pool).await,
            &phoenixd,
            &servers,
            metrics.clone(),
        );

        let result = service.create_verification().await;

        assert!(matches!(result, Err(LnVerificationError::Phoenixd(_))));
        assert_eq!(metrics.dependency_errors(Dependency::Phoenixd), 1);
        assert_eq!(metrics.verification_started(SignupType::Lightning), 0);
    }

    #[sqlx::test]
    async fn background_sync_counts_a_failed_websocket_connection(pool: PgPool) {
        let servers = WiremockServers::start().await;
        // Plain HTTP mock: the websocket upgrade is refused.
        let phoenixd = MockServer::start().await;
        let metrics = Metrics::new();
        let service = service(
            SqlDb::test(pool).await,
            &phoenixd,
            &servers,
            metrics.clone(),
        );

        let result = service.run_background_sync().await;

        assert!(matches!(result, Err(LnVerificationError::Phoenixd(_))));
        assert_eq!(metrics.dependency_errors(Dependency::Phoenixd), 1);
    }
}
