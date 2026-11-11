//! Cached JWKS fetching shared by the ID-token verifiers (`google_verification`,
//! `apple_verification`).
//!
//! Keys are cached for the `Cache-Control: max-age` the provider sends (5 minutes
//! when absent), clamped to between 1 minute and 1 day. Only one fetch runs at a
//! time, and it runs without holding the cache lock, so lookups of unexpired
//! cached keys never wait for the provider. A fetch runs in its own task, so it
//! finishes and is recorded even if the request that started it is cancelled.
//!
//! - A token whose `kid` is not in the cached set triggers one forced refetch,
//!   at most once per minute, so unknown `kid`s cannot be used to hammer the
//!   provider.
//! - When the keys are missing or expired and a fetch fails, the next fetch
//!   waits [`FAILED_FETCH_BACKOFF`]. Requests in that window fail as unavailable
//!   right away instead of each waiting for its own fetch.
//! - Within a minute of a failed fetch, a token with an unknown `kid` is
//!   reported as unavailable, not as invalid: the provider may have rotated to a
//!   key we could not fetch. Tokens signed with an unexpired cached key keep
//!   verifying.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use jsonwebtoken::jwk::{Jwk, JwkSet};
use reqwest::header::CACHE_CONTROL;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};
use url::Url;

const DEFAULT_JWKS_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MIN_JWKS_CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_JWKS_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub(crate) const MIN_FORCED_JWKS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// How long a failed fetch is not retried when the keys are missing or expired.
const FAILED_FETCH_BACKOFF: Duration = Duration::from_secs(5);

/// The JWKS endpoint could not be fetched or did not return a valid key set.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("JWKS unavailable")]
pub struct JwksUnavailable;

#[derive(Clone, Debug)]
pub struct JwksCache {
    http_client: reqwest::Client,
    jwks_url: Url,
    /// Held only to read or replace the key set, never during a fetch.
    keys: Arc<RwLock<Option<CachedJwks>>>,
    /// Held during a fetch, so only one fetch runs at a time.
    last_fetch: Arc<Mutex<Option<FetchAttempt>>>,
}

impl JwksCache {
    pub fn new(jwks_url: Url) -> Self {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(crate::shared::DEPENDENCY_REQUEST_TIMEOUT)
            .build()
            .expect("Reqwest client configuration should be valid");
        Self {
            http_client,
            jwks_url,
            keys: Arc::new(RwLock::new(None)),
            last_fetch: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the key with the given `kid`, refetching the key set once if the
    /// cached set does not contain it. `Ok(None)` means the key is unknown.
    pub async fn find_key(&self, kid: &str) -> Result<Option<Jwk>, JwksUnavailable> {
        let jwks = self.current_keys().await?;
        if let Some(jwk) = find_jwk(&jwks, kid) {
            return Ok(Some(jwk.clone()));
        }
        let refreshed_jwks = self.refresh_for_unknown_kid().await?;
        Ok(find_jwk(&refreshed_jwks, kid).cloned())
    }

    /// Returns the cached key set, fetching it first if it is missing or expired.
    async fn current_keys(&self) -> Result<Arc<JwkSet>, JwksUnavailable> {
        if let Some(jwks) = self.unexpired_keys().await {
            return Ok(jwks);
        }
        let last_fetch = self.last_fetch.clone().lock_owned().await;
        // Another request may have fetched the keys while this one waited.
        if let Some(jwks) = self.unexpired_keys().await {
            return Ok(jwks);
        }
        if let Some(attempt) = *last_fetch
            && !attempt.succeeded
            && attempt.finished_at.elapsed() < FAILED_FETCH_BACKOFF
        {
            return Err(JwksUnavailable);
        }
        self.fetch_and_store(last_fetch).await
    }

    /// Refetches the key set for a token whose `kid` is not cached, at most once
    /// per [`MIN_FORCED_JWKS_REFRESH_INTERVAL`].
    async fn refresh_for_unknown_kid(&self) -> Result<Arc<JwkSet>, JwksUnavailable> {
        let last_fetch = self.last_fetch.clone().lock_owned().await;
        if let Some(attempt) = *last_fetch
            && attempt.finished_at.elapsed() < MIN_FORCED_JWKS_REFRESH_INTERVAL
        {
            // Throttled. If the last fetch failed, the provider may have rotated
            // to a key we cannot see yet, so the token is not reported as invalid.
            if !attempt.succeeded {
                return Err(JwksUnavailable);
            }
            return self.cached_keys().await.ok_or(JwksUnavailable);
        }
        self.fetch_and_store(last_fetch).await
    }

    /// Fetches the key set and stores it on success. The caller passes in the
    /// `last_fetch` lock, so only one fetch runs at a time. The fetch runs in its
    /// own task that owns the lock, so a cancelled request cannot drop it halfway
    /// and skip recording the attempt.
    async fn fetch_and_store(
        &self,
        mut last_fetch: OwnedMutexGuard<Option<FetchAttempt>>,
    ) -> Result<Arc<JwkSet>, JwksUnavailable> {
        let this = self.clone();
        tokio::spawn(async move {
            let result = this.fetch().await;
            // Timed when the fetch ends, so requests that waited behind a slow
            // failed fetch still see it as recent and don't start their own.
            *last_fetch = Some(FetchAttempt {
                finished_at: Instant::now(),
                succeeded: result.is_ok(),
            });
            let fetched = result?;
            let jwks = fetched.jwks.clone();
            *this.keys.write().await = Some(fetched);
            Ok(jwks)
        })
        .await
        .map_err(|_| JwksUnavailable)?
    }

    async fn unexpired_keys(&self) -> Option<Arc<JwkSet>> {
        let keys = self.keys.read().await;
        keys.as_ref()
            .filter(|cached| !cached.is_expired())
            .map(|cached| cached.jwks.clone())
    }

    async fn cached_keys(&self) -> Option<Arc<JwkSet>> {
        let keys = self.keys.read().await;
        keys.as_ref().map(|cached| cached.jwks.clone())
    }

    async fn fetch(&self) -> Result<CachedJwks, JwksUnavailable> {
        self.try_fetch().await.map_err(|error| {
            tracing::warn!(jwks_url = %self.jwks_url, error = %error, "JWKS fetch failed");
            JwksUnavailable
        })
    }

    async fn try_fetch(&self) -> Result<CachedJwks, reqwest::Error> {
        let response = self.http_client.get(self.jwks_url.clone()).send().await?;
        let ttl = response
            .headers()
            .get(CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .and_then(cache_control_max_age)
            .unwrap_or(DEFAULT_JWKS_CACHE_TTL)
            .clamp(MIN_JWKS_CACHE_TTL, MAX_JWKS_CACHE_TTL);
        let jwks = response.error_for_status()?.json::<JwkSet>().await?;

        Ok(CachedJwks::new(jwks, ttl))
    }

    /// Replaces the cached key set, e.g. to simulate a stale cache after key rotation.
    #[cfg(test)]
    pub(crate) async fn seed_for_test(
        &self,
        jwks: JwkSet,
        fetched_at: Instant,
        expires_at: Instant,
    ) {
        *self.keys.write().await = Some(CachedJwks {
            jwks: Arc::new(jwks),
            expires_at,
        });
        *self.last_fetch.lock().await = Some(FetchAttempt {
            finished_at: fetched_at,
            succeeded: true,
        });
    }

    /// Records a failed fetch that finished at `finished_at`.
    #[cfg(test)]
    pub(crate) async fn seed_failed_fetch_for_test(&self, finished_at: Instant) {
        *self.last_fetch.lock().await = Some(FetchAttempt {
            finished_at,
            succeeded: false,
        });
    }
}

/// Resolve a provider's JWKS URL, honouring an environment variable override.
///
/// Panics on an invalid override so a misconfiguration fails at startup
/// rather than surfacing as verification errors at runtime.
pub fn jwks_url_from_env_or_default(env_var: &str, default_url: &str) -> Url {
    let Ok(value) = std::env::var(env_var) else {
        return Url::parse(default_url).expect("Default JWKS URL is valid");
    };
    let url = Url::parse(&value)
        .unwrap_or_else(|error| panic!("{env_var} must be a valid URL, got '{value}': {error}"));
    tracing::info!(jwks_url = %url, "Using JWKS URL override from {env_var}");
    url
}

#[derive(Debug)]
struct CachedJwks {
    jwks: Arc<JwkSet>,
    expires_at: Instant,
}

impl CachedJwks {
    fn new(jwks: JwkSet, ttl: Duration) -> Self {
        Self {
            jwks: Arc::new(jwks),
            expires_at: Instant::now() + ttl,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

#[derive(Clone, Copy, Debug)]
struct FetchAttempt {
    finished_at: Instant,
    succeeded: bool,
}

fn find_jwk<'a>(jwks: &'a JwkSet, kid: &str) -> Option<&'a Jwk> {
    jwks.keys
        .iter()
        .find(|jwk| jwk.common.key_id.as_deref() == Some(kid))
}

fn cache_control_max_age(value: &str) -> Option<Duration> {
    value.split(',').find_map(|directive| {
        let (name, value) = directive.trim().split_once('=')?;
        if name.eq_ignore_ascii_case("max-age") {
            value
                .trim()
                .trim_matches('"')
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn jwk_with_kid(kid: &str) -> Jwk {
        serde_json::from_value(json!({
            "kty": "RSA",
            "kid": kid,
            "use": "sig",
            "alg": "RS256",
            "n": "sXchDaQebHnPiGvyDOAT4saGEUetSyo9MKLOoWFsueri23bOdgWp4Dy1WlUzewbgBHod5pcM9H95GQRV3JDXboIRROSBigeC5yjU1hGzHHyXss8UDprecbAYxknTcQkhslANGRUZmdTOQ5qTRsLAt6BTYuyvVRdhS8exSZEy_c4gs_7svlJJQ4H9_NxsiIoLwAEk7-Q3UXERGYw_75IDrGA84-lA_-Ct4eTlXHBIY2EaV7t7LjJaynVJCpkv4LKjTTAumiGUIuQhrNhZLuF_RJLqHpM2kgWFLU7-VTdL1VbC2tejvcI2BlMkEpk1BzBZI0KQB0GaDWFLN-aEAw3vRw",
            "e": "AQAB"
        }))
        .expect("test JWK should deserialize")
    }

    async fn jwks_server_with_kid(kid: &str, expected_fetches: u64) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/keys"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", "public, max-age=3600")
                    .set_body_json(json!({ "keys": [jwk_with_kid(kid)] })),
            )
            .expect(expected_fetches)
            .mount(&server)
            .await;
        server
    }

    fn cache_for_server(server: &MockServer) -> JwksCache {
        JwksCache::new(format!("{}/keys", server.uri()).parse().unwrap())
    }

    #[test]
    fn test_cache_control_max_age() {
        assert_eq!(
            cache_control_max_age("public, max-age=123, must-revalidate"),
            Some(Duration::from_secs(123))
        );
        assert_eq!(
            cache_control_max_age("public, max-age=\"456\""),
            Some(Duration::from_secs(456))
        );
        assert_eq!(cache_control_max_age("no-cache"), None);
    }

    #[tokio::test]
    async fn test_caches_key_set_between_lookups() {
        let server = jwks_server_with_kid("kid-a", 1).await;
        let cache = cache_for_server(&server);

        assert!(cache.find_key("kid-a").await.unwrap().is_some());
        assert!(cache.find_key("kid-a").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_unknown_kid_refetch_is_throttled() {
        let server = jwks_server_with_kid("kid-a", 1).await;
        let cache = cache_for_server(&server);

        assert!(cache.find_key("unknown-kid").await.unwrap().is_none());
        assert!(cache.find_key("other-unknown-kid").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_unknown_kid_refetches_after_throttle_window() {
        let server = jwks_server_with_kid("kid-b", 1).await;
        let cache = cache_for_server(&server);
        cache
            .seed_for_test(
                seeded_jwks("kid-a"),
                Instant::now() - MIN_FORCED_JWKS_REFRESH_INTERVAL - Duration::from_secs(1),
                Instant::now() + Duration::from_secs(3600),
            )
            .await;

        assert!(cache.find_key("kid-b").await.unwrap().is_some());
    }

    fn seeded_jwks(kid: &str) -> JwkSet {
        JwkSet {
            keys: vec![jwk_with_kid(kid)],
        }
    }

    async fn failing_jwks_server(delay: Duration, expected_fetches: u64) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/keys"))
            .respond_with(ResponseTemplate::new(503).set_delay(delay))
            .expect(expected_fetches)
            .mount(&server)
            .await;
        server
    }

    async fn wait_for_requests(server: &MockServer, count: usize) {
        for _ in 0..100 {
            if server.received_requests().await.unwrap().len() >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("JWKS server did not receive {count} request(s)");
    }

    #[tokio::test]
    async fn test_failed_forced_refresh_reports_unknown_kid_as_unavailable() {
        let server = failing_jwks_server(Duration::ZERO, 1).await;
        let cache = cache_for_server(&server);
        cache
            .seed_for_test(
                seeded_jwks("kid-a"),
                Instant::now() - MIN_FORCED_JWKS_REFRESH_INTERVAL - Duration::from_secs(1),
                Instant::now() + Duration::from_secs(3600),
            )
            .await;

        // The refresh fails, so the unknown kid is unavailable, not invalid.
        assert!(cache.find_key("unknown-kid").await.is_err());
        // Throttled: no second fetch, still unavailable.
        assert!(cache.find_key("other-unknown-kid").await.is_err());
        // Cached keys keep working.
        assert!(cache.find_key("kid-a").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_failed_fetch_is_not_retried_during_backoff() {
        let server = failing_jwks_server(Duration::ZERO, 1).await;
        let cache = cache_for_server(&server);

        assert!(cache.find_key("kid-a").await.is_err());
        assert!(cache.find_key("kid-a").await.is_err());
    }

    #[tokio::test]
    async fn test_failed_fetch_is_retried_after_backoff() {
        let server = jwks_server_with_kid("kid-a", 1).await;
        let cache = cache_for_server(&server);
        cache
            .seed_failed_fetch_for_test(
                Instant::now() - FAILED_FETCH_BACKOFF - Duration::from_secs(1),
            )
            .await;

        assert!(cache.find_key("kid-a").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_concurrent_lookups_share_one_failed_fetch() {
        let server = failing_jwks_server(Duration::from_millis(300), 1).await;
        let cache = cache_for_server(&server);

        let lookups: Vec<_> = (0..5)
            .map(|_| {
                let cache = cache.clone();
                tokio::spawn(async move { cache.find_key("kid-a").await })
            })
            .collect();
        for lookup in lookups {
            assert!(lookup.await.unwrap().is_err());
        }
    }

    #[tokio::test]
    async fn test_forced_refresh_does_not_block_cached_lookups() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/keys"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "keys": [jwk_with_kid("kid-a")] }))
                    .set_delay(Duration::from_secs(2)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let cache = cache_for_server(&server);
        cache
            .seed_for_test(
                seeded_jwks("kid-a"),
                Instant::now() - MIN_FORCED_JWKS_REFRESH_INTERVAL - Duration::from_secs(1),
                Instant::now() + Duration::from_secs(3600),
            )
            .await;

        let forced = {
            let cache = cache.clone();
            tokio::spawn(async move { cache.find_key("unknown-kid").await })
        };
        wait_for_requests(&server, 1).await;

        let started = Instant::now();
        assert!(cache.find_key("kid-a").await.unwrap().is_some());
        assert!(started.elapsed() < Duration::from_secs(1));

        assert!(forced.await.unwrap().unwrap().is_none());
    }

    #[tokio::test]
    async fn test_cancelled_forced_refresh_still_counts_towards_throttle() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/keys"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "keys": [jwk_with_kid("kid-a")] }))
                    .set_delay(Duration::from_millis(300)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let cache = cache_for_server(&server);
        cache
            .seed_for_test(
                seeded_jwks("kid-a"),
                Instant::now() - MIN_FORCED_JWKS_REFRESH_INTERVAL - Duration::from_secs(1),
                Instant::now() + Duration::from_secs(3600),
            )
            .await;

        // A client that disconnects mid-fetch must not reset the throttle.
        for _ in 0..3 {
            let lookup = {
                let cache = cache.clone();
                tokio::spawn(async move { cache.find_key("unknown-kid").await })
            };
            wait_for_requests(&server, 1).await;
            lookup.abort();
        }

        assert!(cache.find_key("unknown-kid").await.unwrap().is_none());
    }

    async fn jwks_server_with_cache_control(cache_control: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/keys"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", cache_control)
                    .set_body_json(json!({ "keys": [jwk_with_kid("kid-a")] })),
            )
            .expect(1)
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn test_zero_max_age_is_cached_for_the_minimum_ttl() {
        let server = jwks_server_with_cache_control("max-age=0").await;
        let cache = cache_for_server(&server);

        assert!(cache.find_key("kid-a").await.unwrap().is_some());
        assert!(cache.find_key("kid-a").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_huge_max_age_does_not_overflow() {
        let server = jwks_server_with_cache_control("max-age=18446744073709551615").await;
        let cache = cache_for_server(&server);

        assert!(cache.find_key("kid-a").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_fetch_failure_is_unavailable() {
        let server = MockServer::start().await;
        let cache = cache_for_server(&server);

        assert!(cache.find_key("kid-a").await.is_err());
    }
}
