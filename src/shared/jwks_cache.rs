//! Cached JWKS fetching shared by the ID-token verifiers (`google_verification`,
//! `apple_verification`).
//!
//! Keys are cached for the `Cache-Control: max-age` the provider sends (5 minutes
//! when absent). A token whose `kid` is not in the cached set triggers one forced
//! refetch, throttled to one per minute so unknown `kid`s cannot be used to hammer
//! the provider.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use jsonwebtoken::jwk::{Jwk, JwkSet};
use reqwest::header::CACHE_CONTROL;
use tokio::sync::RwLock;
use url::Url;

const DEFAULT_JWKS_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
pub(crate) const MIN_FORCED_JWKS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// The JWKS endpoint could not be fetched or did not return a valid key set.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("JWKS unavailable")]
pub struct JwksUnavailable;

#[derive(Clone, Debug)]
pub struct JwksCache {
    http_client: reqwest::Client,
    jwks_url: Url,
    cache: Arc<RwLock<Option<CachedJwks>>>,
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
            cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Returns the key with the given `kid`, refetching the key set once if the
    /// cached set does not contain it. `Ok(None)` means the key is unknown.
    pub async fn find_key(&self, kid: &str) -> Result<Option<Jwk>, JwksUnavailable> {
        let jwks = self.get(false).await?;
        if let Some(jwk) = find_jwk(&jwks, kid) {
            return Ok(Some(jwk.clone()));
        }
        let refreshed_jwks = self.get(true).await?;
        Ok(find_jwk(&refreshed_jwks, kid).cloned())
    }

    async fn get(&self, force_refresh: bool) -> Result<Arc<JwkSet>, JwksUnavailable> {
        if !force_refresh {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref()
                && !cached.is_expired()
            {
                return Ok(cached.jwks.clone());
            }
        } else {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref()
                && cached.is_forced_refresh_throttled()
            {
                return Ok(cached.jwks.clone());
            }
        }

        let mut cache = self.cache.write().await;
        if !force_refresh
            && let Some(cached) = cache.as_ref()
            && !cached.is_expired()
        {
            return Ok(cached.jwks.clone());
        }
        if force_refresh
            && let Some(cached) = cache.as_ref()
            && cached.is_forced_refresh_throttled()
        {
            return Ok(cached.jwks.clone());
        }

        let response = self
            .http_client
            .get(self.jwks_url.clone())
            .send()
            .await
            .map_err(|_| JwksUnavailable)?;
        let ttl = response
            .headers()
            .get(CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .and_then(cache_control_max_age)
            .unwrap_or(DEFAULT_JWKS_CACHE_TTL);
        let jwks = response
            .error_for_status()
            .map_err(|_| JwksUnavailable)?
            .json::<JwkSet>()
            .await
            .map_err(|_| JwksUnavailable)?;

        let cached = CachedJwks::new(jwks, ttl);
        let jwks = cached.jwks.clone();
        *cache = Some(cached);
        Ok(jwks)
    }

    /// Replaces the cached key set, e.g. to simulate a stale cache after key rotation.
    #[cfg(test)]
    pub(crate) async fn seed_for_test(
        &self,
        jwks: JwkSet,
        fetched_at: Instant,
        expires_at: Instant,
    ) {
        *self.cache.write().await = Some(CachedJwks {
            jwks: Arc::new(jwks),
            fetched_at,
            expires_at,
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

#[derive(Clone, Debug)]
struct CachedJwks {
    jwks: Arc<JwkSet>,
    fetched_at: Instant,
    expires_at: Instant,
}

impl CachedJwks {
    fn new(jwks: JwkSet, ttl: Duration) -> Self {
        let fetched_at = Instant::now();
        Self {
            jwks: Arc::new(jwks),
            fetched_at,
            expires_at: fetched_at + ttl,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    fn is_forced_refresh_throttled(&self) -> bool {
        self.fetched_at.elapsed() < MIN_FORCED_JWKS_REFRESH_INTERVAL
    }
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
                JwkSet {
                    keys: vec![jwk_with_kid("kid-a")],
                },
                Instant::now() - MIN_FORCED_JWKS_REFRESH_INTERVAL - Duration::from_secs(1),
                Instant::now() + Duration::from_secs(3600),
            )
            .await;

        assert!(cache.find_key("kid-b").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_fetch_failure_is_unavailable() {
        let server = MockServer::start().await;
        let cache = cache_for_server(&server);

        assert!(cache.find_key("kid-a").await.is_err());
    }
}
