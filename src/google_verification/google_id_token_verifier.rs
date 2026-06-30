use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use reqwest::header::CACHE_CONTROL;
use tokio::sync::RwLock;
use url::Url;

const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const DEFAULT_JWKS_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const GOOGLE_JWKS_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_FORCED_JWKS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const MAX_ID_TOKEN_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedGoogleIdentity {
    pub issuer: String,
    pub subject: String,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum GoogleIdTokenVerificationError {
    #[error("invalid Google ID token")]
    Invalid,

    #[error("Google verifier unavailable")]
    DependencyUnavailable,
}

#[async_trait]
pub trait GoogleIdTokenVerifier: Send + Sync + std::fmt::Debug {
    async fn verify(
        &self,
        id_token: &str,
    ) -> Result<VerifiedGoogleIdentity, GoogleIdTokenVerificationError>;
}

#[derive(Clone, Debug)]
pub struct GoogleJwksIdTokenVerifier {
    google_client_id: String,
    jwks_cache: GoogleJwksCache,
}

impl GoogleJwksIdTokenVerifier {
    pub fn new(google_client_id: String) -> Self {
        Self::with_jwks_url(
            google_client_id,
            Url::parse(GOOGLE_JWKS_URL).expect("Google JWKS URL is valid"),
        )
    }

    fn with_jwks_url(google_client_id: String, jwks_url: Url) -> Self {
        Self {
            google_client_id,
            jwks_cache: GoogleJwksCache::new(jwks_url),
        }
    }
}

#[async_trait]
impl GoogleIdTokenVerifier for GoogleJwksIdTokenVerifier {
    async fn verify(
        &self,
        id_token: &str,
    ) -> Result<VerifiedGoogleIdentity, GoogleIdTokenVerificationError> {
        if id_token.trim().is_empty() || id_token.len() > MAX_ID_TOKEN_BYTES {
            return Err(GoogleIdTokenVerificationError::Invalid);
        }

        let header =
            decode_header(id_token).map_err(|_| GoogleIdTokenVerificationError::Invalid)?;
        if header.alg != Algorithm::RS256 {
            return Err(GoogleIdTokenVerificationError::Invalid);
        }
        let kid = header.kid.ok_or(GoogleIdTokenVerificationError::Invalid)?;

        let jwks = self.jwks_cache.get(false).await?;
        let jwk = match find_jwk(&jwks, &kid).cloned() {
            Some(jwk) => jwk,
            None => {
                let refreshed_jwks = self.jwks_cache.get(true).await?;
                find_jwk(&refreshed_jwks, &kid)
                    .cloned()
                    .ok_or(GoogleIdTokenVerificationError::Invalid)?
            }
        };

        let decoding_key = DecodingKey::from_jwk(&jwk)
            .map_err(|_| GoogleIdTokenVerificationError::DependencyUnavailable)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[self.google_client_id.as_str()]);
        validation.set_issuer(&["accounts.google.com", "https://accounts.google.com"]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);

        let token = decode::<GoogleClaims>(id_token, &decoding_key, &validation)
            .map_err(|_| GoogleIdTokenVerificationError::Invalid)?;

        if token.claims.sub.trim().is_empty() {
            return Err(GoogleIdTokenVerificationError::Invalid);
        }

        Ok(VerifiedGoogleIdentity {
            issuer: token.claims.iss,
            subject: token.claims.sub,
        })
    }
}

#[derive(Clone, Debug)]
struct GoogleJwksCache {
    http_client: reqwest::Client,
    jwks_url: Url,
    cache: Arc<RwLock<Option<CachedJwks>>>,
}

impl GoogleJwksCache {
    fn new(jwks_url: Url) -> Self {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(GOOGLE_JWKS_REQUEST_TIMEOUT)
            .build()
            .expect("Reqwest client configuration should be valid");
        Self {
            http_client,
            jwks_url,
            cache: Arc::new(RwLock::new(None)),
        }
    }

    async fn get(
        &self,
        force_refresh: bool,
    ) -> Result<Arc<JwkSet>, GoogleIdTokenVerificationError> {
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
            .map_err(|_| GoogleIdTokenVerificationError::DependencyUnavailable)?;
        let ttl = response
            .headers()
            .get(CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .and_then(cache_control_max_age)
            .unwrap_or(DEFAULT_JWKS_CACHE_TTL);
        let jwks = response
            .error_for_status()
            .map_err(|_| GoogleIdTokenVerificationError::DependencyUnavailable)?
            .json::<JwkSet>()
            .await
            .map_err(|_| GoogleIdTokenVerificationError::DependencyUnavailable)?;

        let cached = CachedJwks::new(jwks, ttl);
        let jwks = cached.jwks.clone();
        *cache = Some(cached);
        Ok(jwks)
    }
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

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct GoogleClaims {
    iss: String,
    sub: String,
    aud: String,
    exp: usize,
}

fn find_jwk<'a>(jwks: &'a JwkSet, kid: &str) -> Option<&'a jsonwebtoken::jwk::Jwk> {
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
impl GoogleJwksIdTokenVerifier {
    pub(crate) fn for_test(google_client_id: String, jwks_url: Url) -> Self {
        Self::with_jwks_url(google_client_id, jwks_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Instant};

    use base64::prelude::{BASE64_STANDARD, Engine as _};
    use jsonwebtoken::{EncodingKey, Header, encode, jwk::Jwk};
    use serde::Serialize;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    const TEST_GOOGLE_CLIENT_ID: &str = "test-google-client-id.apps.googleusercontent.com";
    const TEST_KID: &str = "test-kid";
    const TEST_RSA_PRIVATE_KEY_DER_BASE64: &str = "MIIEowIBAAKCAQEAhJMAK09DfEZUScZqfJJjt7JX+mt+2Ik81el4C7M8OmtLEVyhT0HFXkV93/fB3gUEkPt+mmxZ4EJG7O5DR9Wi9Z4S0qJalrjImoqwvjtPK10E3EhS4Ma+G8KjjBB+hBo9bEC9EtbeZT+mOlVqAM81dHbi/tw77nGQOveJcevRgUpUjOealBerKy2zjJzp9mvIiW1eTS1bX1nMKF3OpUpyd3RVXdPV9OvdJxDZqWv+7MQK1BO8CEVNKcvLUN6O2NgyZwW9HnA7RMxQbEaUyCC8NjcjyMtH54HCtbqseozt1W2jfKc009b8HZt6sDDrxqyjohVs7bD3ubIJ29n2+Pgh4wIDAQABAoIBABipO6vSx8vzTTSYCzD3DkOaklEL9AGVrdJg5qrOgZKgaMtm/r6+jldV9+9UqCSDrHDHx6o0I5fa3FSwkaVoMTMdX4T9HHrTDsXorK4GXFjFqeTMM1aKwcxqLYAdhVtPgkOD22gIvj/5UhOh1eEmqlvqzZj5INDfISRG7bNaWZOCGqQBqS5ncM+pOQAXgkR5A4SdSbQuT/aGd+kvneui09BZAQt9dMiXg+7dzDn+szjfJjcij1MqDPOJvnPOhmQwnUkHh+dvEbBbOMVTIe/smFxuaTTMx9kz9nOsdpmaXK1LOywZNtR/5lEIJqSSD2z8hH1rtTbHu7N9IRMyDwIkibECgYEAuTHcVYBGSEOdWnBAOA+sQqpDQdzeR+8pLGhbOc0altyBCi4suAeyke5joh01v4ryBQxsDGDOdO9/fknWFLXxAq2aHX0p+CtKugjs+BOPUBv+kaIiV8FM860XDXQDhoEkVATsNpG3CldpztWxjnvmBcnUgLHmFBzsKjg9oLE9Q4sCgYEAt0LYw1f3ygdrNTIbQRLXIEkxaX7cZZcSOqrqeOOtwkw1I3zK/6UbS80lWdKU2A32lIaawxWIm4HJDoXIvZzIHmhTZFmrCMFVNrRomZimKdQm2vNRQNP/sDGW07X8KGb03P6kO08GHpET1qJvqDylB23zt36c3rcGfzm1nZwihgkCgYA9lekxvcChk2qmgqG4gu3EFZ7cLjj1LwFANUvxAtYOyTFYU1antFeb0+zqIlCXa/tj1mewDhlaJbL+Kku5A3AsddLEb7UfRDZLe2Bidw63kzeq8oH9MNkIR81cufHaLuQH1MNAumBmXf9fuwya13T9A8tZKM/cbGnU+HL2FzrKVQKBgQCJz1/4DffNWiTZnPN3zPYvVjstLPQKBT/1FEA8ZmJtQSeYpyh0dDGBoCRdVokNq/pomIxa9Z+D6WZLYHmjdPncO/Gx/egrLk+pUqNyFaOmwt3xOpY4nPOjCLd2P1z++OVcJrVT0Eo2xDxZ5E75AZnMa3eh3jmTFalyFPCpNBeWGQKBgHlYz/HDaLayLFcbo+2XLAKy4m2TWUFEIcdUtlTZPdy2fThe1yM8cS5S3KZUaUoEf2Tdr8CmcXnXrkpHKeDVrhOt6IoecwHHvPFngG1ZRU8hzQ5kKF23pb1OuCIEpBXO6z584c3VIUY8AghMe+Bs09TcALFyoYzUHeYgKl1EBM8v";

    #[derive(Serialize)]
    struct TestClaims {
        iss: String,
        sub: String,
        aud: String,
        exp: usize,
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
    async fn test_verifies_valid_google_like_token() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://accounts.google.com",
            "google-subject",
            future_exp(),
        );

        let identity = verifier.verify(&token).await.unwrap();

        assert_eq!(identity.issuer, "https://accounts.google.com");
        assert_eq!(identity.subject, "google-subject");
    }

    #[tokio::test]
    async fn test_rejects_wrong_audience() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = test_token(
            "other-client-id",
            "https://accounts.google.com",
            "google-subject",
            future_exp(),
        );

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(error, GoogleIdTokenVerificationError::Invalid));
    }

    #[tokio::test]
    async fn test_rejects_wrong_issuer() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://evil.example",
            "google-subject",
            future_exp(),
        );

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(error, GoogleIdTokenVerificationError::Invalid));
    }

    #[tokio::test]
    async fn test_rejects_expired_token() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://accounts.google.com",
            "google-subject",
            past_exp(),
        );

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(error, GoogleIdTokenVerificationError::Invalid));
    }

    #[tokio::test]
    async fn test_rejects_empty_subject() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://accounts.google.com",
            "",
            future_exp(),
        );

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(error, GoogleIdTokenVerificationError::Invalid));
    }

    #[tokio::test]
    async fn test_maps_jwks_fetch_failure_to_dependency_unavailable() {
        let server = MockServer::start().await;
        let verifier = verifier_for_server(&server);
        let token = test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://accounts.google.com",
            "google-subject",
            future_exp(),
        );

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(
            error,
            GoogleIdTokenVerificationError::DependencyUnavailable
        ));
    }

    #[tokio::test]
    async fn test_refreshes_jwks_when_cached_keys_do_not_match_token_kid() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let mut old_jwk = test_jwk();
        old_jwk.common.key_id = Some("old-kid".to_string());
        *verifier.jwks_cache.cache.write().await = Some(CachedJwks {
            jwks: Arc::new(JwkSet {
                keys: vec![old_jwk],
            }),
            fetched_at: Instant::now() - MIN_FORCED_JWKS_REFRESH_INTERVAL - Duration::from_secs(1),
            expires_at: Instant::now() + Duration::from_secs(3600),
        });
        let token = test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://accounts.google.com",
            "google-subject",
            future_exp(),
        );

        let identity = verifier.verify(&token).await.unwrap();

        assert_eq!(identity.subject, "google-subject");
    }

    async fn jwks_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/certs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", "public, max-age=3600")
                    .set_body_json(json!({ "keys": [test_jwk()] })),
            )
            .mount(&server)
            .await;
        server
    }

    fn verifier_for_server(server: &MockServer) -> GoogleJwksIdTokenVerifier {
        GoogleJwksIdTokenVerifier::for_test(
            TEST_GOOGLE_CLIENT_ID.to_string(),
            format!("{}/certs", server.uri()).parse().unwrap(),
        )
    }

    fn test_token(audience: &str, issuer: &str, subject: &str, exp: usize) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        encode(
            &header,
            &TestClaims {
                iss: issuer.to_string(),
                sub: subject.to_string(),
                aud: audience.to_string(),
                exp,
            },
            &EncodingKey::from_rsa_der(&test_private_key_der()),
        )
        .expect("test token should encode")
    }

    fn test_jwk() -> Jwk {
        let encoding_key = EncodingKey::from_rsa_der(&test_private_key_der());
        let mut jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::RS256)
            .expect("test JWK should be built from RSA key");
        jwk.common.key_id = Some(TEST_KID.to_string());
        jwk
    }

    fn test_private_key_der() -> Vec<u8> {
        BASE64_STANDARD
            .decode(TEST_RSA_PRIVATE_KEY_DER_BASE64)
            .expect("test RSA key should decode")
    }

    fn future_exp() -> usize {
        (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize
    }

    fn past_exp() -> usize {
        (chrono::Utc::now() - chrono::Duration::hours(1)).timestamp() as usize
    }
}
