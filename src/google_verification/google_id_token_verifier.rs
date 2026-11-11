use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use url::Url;

use crate::shared::{JwksCache, jwks_url_from_env_or_default};

const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
/// Environment variable that overrides [`GOOGLE_JWKS_URL`]. Intentionally not
/// part of `config.toml` — it exists as an escape hatch (e.g. pointing at a
/// mock during manual testing, or if Google ever moves the endpoint).
const GOOGLE_JWKS_URL_ENV_VAR: &str = "HOMEGATE_GOOGLE_JWKS_URL";
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

#[derive(Clone, Debug)]
pub struct GoogleJwksIdTokenVerifier {
    google_client_id: String,
    jwks_cache: JwksCache,
}

impl GoogleJwksIdTokenVerifier {
    pub fn new(google_client_id: String) -> Self {
        Self::with_jwks_url(
            google_client_id,
            jwks_url_from_env_or_default(GOOGLE_JWKS_URL_ENV_VAR, GOOGLE_JWKS_URL),
        )
    }

    fn with_jwks_url(google_client_id: String, jwks_url: Url) -> Self {
        Self {
            google_client_id,
            jwks_cache: JwksCache::new(jwks_url),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(google_client_id: String, jwks_url: Url) -> Self {
        Self::with_jwks_url(google_client_id, jwks_url)
    }
}

impl GoogleJwksIdTokenVerifier {
    pub async fn verify(
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

        let jwk = self
            .jwks_cache
            .find_key(&kid)
            .await
            .map_err(|_| GoogleIdTokenVerificationError::DependencyUnavailable)?
            .ok_or(GoogleIdTokenVerificationError::Invalid)?;

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

        let GoogleClaims { iss, sub, .. } = token.claims;
        let issuer = if iss == "accounts.google.com" {
            "https://accounts.google.com".to_string()
        } else {
            iss
        };

        Ok(VerifiedGoogleIdentity {
            issuer,
            subject: sub,
        })
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

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    use base64::prelude::{BASE64_STANDARD, Engine as _};
    use jsonwebtoken::{EncodingKey, Header, encode, jwk::Jwk};
    use serde::Serialize;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    pub(crate) const TEST_GOOGLE_CLIENT_ID: &str =
        "test-google-client-id.apps.googleusercontent.com";
    const TEST_KID: &str = "test-kid";
    const TEST_RSA_PRIVATE_KEY_DER_BASE64: &str = "MIIEowIBAAKCAQEAhJMAK09DfEZUScZqfJJjt7JX+mt+2Ik81el4C7M8OmtLEVyhT0HFXkV93/fB3gUEkPt+mmxZ4EJG7O5DR9Wi9Z4S0qJalrjImoqwvjtPK10E3EhS4Ma+G8KjjBB+hBo9bEC9EtbeZT+mOlVqAM81dHbi/tw77nGQOveJcevRgUpUjOealBerKy2zjJzp9mvIiW1eTS1bX1nMKF3OpUpyd3RVXdPV9OvdJxDZqWv+7MQK1BO8CEVNKcvLUN6O2NgyZwW9HnA7RMxQbEaUyCC8NjcjyMtH54HCtbqseozt1W2jfKc009b8HZt6sDDrxqyjohVs7bD3ubIJ29n2+Pgh4wIDAQABAoIBABipO6vSx8vzTTSYCzD3DkOaklEL9AGVrdJg5qrOgZKgaMtm/r6+jldV9+9UqCSDrHDHx6o0I5fa3FSwkaVoMTMdX4T9HHrTDsXorK4GXFjFqeTMM1aKwcxqLYAdhVtPgkOD22gIvj/5UhOh1eEmqlvqzZj5INDfISRG7bNaWZOCGqQBqS5ncM+pOQAXgkR5A4SdSbQuT/aGd+kvneui09BZAQt9dMiXg+7dzDn+szjfJjcij1MqDPOJvnPOhmQwnUkHh+dvEbBbOMVTIe/smFxuaTTMx9kz9nOsdpmaXK1LOywZNtR/5lEIJqSSD2z8hH1rtTbHu7N9IRMyDwIkibECgYEAuTHcVYBGSEOdWnBAOA+sQqpDQdzeR+8pLGhbOc0altyBCi4suAeyke5joh01v4ryBQxsDGDOdO9/fknWFLXxAq2aHX0p+CtKugjs+BOPUBv+kaIiV8FM860XDXQDhoEkVATsNpG3CldpztWxjnvmBcnUgLHmFBzsKjg9oLE9Q4sCgYEAt0LYw1f3ygdrNTIbQRLXIEkxaX7cZZcSOqrqeOOtwkw1I3zK/6UbS80lWdKU2A32lIaawxWIm4HJDoXIvZzIHmhTZFmrCMFVNrRomZimKdQm2vNRQNP/sDGW07X8KGb03P6kO08GHpET1qJvqDylB23zt36c3rcGfzm1nZwihgkCgYA9lekxvcChk2qmgqG4gu3EFZ7cLjj1LwFANUvxAtYOyTFYU1antFeb0+zqIlCXa/tj1mewDhlaJbL+Kku5A3AsddLEb7UfRDZLe2Bidw63kzeq8oH9MNkIR81cufHaLuQH1MNAumBmXf9fuwya13T9A8tZKM/cbGnU+HL2FzrKVQKBgQCJz1/4DffNWiTZnPN3zPYvVjstLPQKBT/1FEA8ZmJtQSeYpyh0dDGBoCRdVokNq/pomIxa9Z+D6WZLYHmjdPncO/Gx/egrLk+pUqNyFaOmwt3xOpY4nPOjCLd2P1z++OVcJrVT0Eo2xDxZ5E75AZnMa3eh3jmTFalyFPCpNBeWGQKBgHlYz/HDaLayLFcbo+2XLAKy4m2TWUFEIcdUtlTZPdy2fThe1yM8cS5S3KZUaUoEf2Tdr8CmcXnXrkpHKeDVrhOt6IoecwHHvPFngG1ZRU8hzQ5kKF23pb1OuCIEpBXO6z584c3VIUY8AghMe+Bs09TcALFyoYzUHeYgKl1EBM8v";

    #[derive(Serialize)]
    struct TestClaims {
        iss: String,
        sub: String,
        aud: String,
        exp: usize,
    }

    pub(crate) async fn jwks_server() -> MockServer {
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

    pub(crate) fn jwks_url(server: &MockServer) -> Url {
        format!("{}/certs", server.uri()).parse().unwrap()
    }

    pub(crate) fn valid_token(subject: &str) -> String {
        test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://accounts.google.com",
            subject,
            future_exp(),
        )
    }

    pub(crate) fn wrong_audience_token() -> String {
        test_token(
            "other-client-id",
            "https://accounts.google.com",
            "google-subject",
            future_exp(),
        )
    }

    pub(crate) fn wrong_issuer_token() -> String {
        test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://evil.example",
            "google-subject",
            future_exp(),
        )
    }

    pub(crate) fn expired_token() -> String {
        test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://accounts.google.com",
            "google-subject",
            past_exp(),
        )
    }

    pub(crate) fn empty_subject_token() -> String {
        test_token(
            TEST_GOOGLE_CLIENT_ID,
            "https://accounts.google.com",
            "",
            future_exp(),
        )
    }

    pub(crate) fn bare_issuer_token(subject: &str) -> String {
        test_token(
            TEST_GOOGLE_CLIENT_ID,
            "accounts.google.com",
            subject,
            future_exp(),
        )
    }

    pub(crate) fn test_jwk() -> Jwk {
        let encoding_key = EncodingKey::from_rsa_der(&test_private_key_der());
        let mut jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::RS256)
            .expect("test JWK should be built from RSA key");
        jwk.common.key_id = Some(TEST_KID.to_string());
        jwk
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use jsonwebtoken::jwk::JwkSet;
    use test_support::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    use crate::shared::jwks_cache::MIN_FORCED_JWKS_REFRESH_INTERVAL;

    #[tokio::test]
    async fn test_verifies_valid_google_like_token() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = valid_token("google-subject");

        let identity = verifier.verify(&token).await.unwrap();

        assert_eq!(identity.issuer, "https://accounts.google.com");
        assert_eq!(identity.subject, "google-subject");
    }

    #[tokio::test]
    async fn test_canonicalizes_bare_google_issuer() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = bare_issuer_token("google-subject");

        let identity = verifier.verify(&token).await.unwrap();

        assert_eq!(identity.issuer, "https://accounts.google.com");
        assert_eq!(identity.subject, "google-subject");
    }

    #[tokio::test]
    async fn test_rejects_wrong_audience() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = wrong_audience_token();

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(error, GoogleIdTokenVerificationError::Invalid));
    }

    #[tokio::test]
    async fn test_rejects_wrong_issuer() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = wrong_issuer_token();

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(error, GoogleIdTokenVerificationError::Invalid));
    }

    #[tokio::test]
    async fn test_rejects_expired_token() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = expired_token();

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(error, GoogleIdTokenVerificationError::Invalid));
    }

    #[tokio::test]
    async fn test_rejects_empty_subject() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = empty_subject_token();

        let error = verifier.verify(&token).await.unwrap_err();

        assert!(matches!(error, GoogleIdTokenVerificationError::Invalid));
    }

    #[tokio::test]
    async fn test_maps_jwks_fetch_failure_to_dependency_unavailable() {
        let server = MockServer::start().await;
        let verifier = verifier_for_server(&server);
        let token = valid_token("google-subject");

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
        verifier
            .jwks_cache
            .seed_for_test(
                JwkSet {
                    keys: vec![old_jwk],
                },
                Instant::now() - MIN_FORCED_JWKS_REFRESH_INTERVAL - Duration::from_secs(1),
                Instant::now() + Duration::from_secs(3600),
            )
            .await;
        let token = valid_token("google-subject");

        let identity = verifier.verify(&token).await.unwrap();

        assert_eq!(identity.subject, "google-subject");
    }

    #[tokio::test]
    async fn test_unknown_kid_with_failed_refresh_is_dependency_unavailable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let verifier = verifier_for_server(&server);
        let mut old_jwk = test_jwk();
        old_jwk.common.key_id = Some("old-kid".to_string());
        verifier
            .jwks_cache
            .seed_for_test(
                JwkSet {
                    keys: vec![old_jwk],
                },
                Instant::now() - MIN_FORCED_JWKS_REFRESH_INTERVAL - Duration::from_secs(1),
                Instant::now() + Duration::from_secs(3600),
            )
            .await;

        // The provider may have rotated keys, so this is not an invalid token.
        let error = verifier
            .verify(&valid_token("google-subject"))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            GoogleIdTokenVerificationError::DependencyUnavailable
        ));
    }

    fn verifier_for_server(server: &MockServer) -> GoogleJwksIdTokenVerifier {
        GoogleJwksIdTokenVerifier::for_test(TEST_GOOGLE_CLIENT_ID.to_string(), jwks_url(server))
    }
}
