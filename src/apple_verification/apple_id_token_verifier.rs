use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use url::Url;

use crate::shared::{JwksCache, jwks_url_from_env_or_default};

const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";
/// Environment variable that overrides [`APPLE_JWKS_URL`]. Intentionally not
/// part of `config.toml` — it exists as an escape hatch (e.g. pointing at a
/// mock during manual testing, or if Apple ever moves the endpoint).
const APPLE_JWKS_URL_ENV_VAR: &str = "HOMEGATE_APPLE_JWKS_URL";
const APPLE_ISSUER: &str = "https://appleid.apple.com";
const MAX_ID_TOKEN_BYTES: usize = 16 * 1024;
/// Clock skew tolerated when checking `exp` and `iat`.
const CLOCK_SKEW_LEEWAY_SECS: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedAppleIdentity {
    pub issuer: String,
    pub subject: String,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum AppleIdTokenVerificationError {
    #[error("invalid Apple ID token")]
    Invalid,

    #[error("Apple verifier unavailable")]
    DependencyUnavailable,
}

#[derive(Clone, Debug)]
pub struct AppleJwksIdTokenVerifier {
    allowed_audiences: Vec<String>,
    jwks_cache: JwksCache,
}

impl AppleJwksIdTokenVerifier {
    pub fn new(allowed_audiences: Vec<String>) -> Self {
        Self::with_jwks_url(
            allowed_audiences,
            jwks_url_from_env_or_default(APPLE_JWKS_URL_ENV_VAR, APPLE_JWKS_URL),
        )
    }

    fn with_jwks_url(allowed_audiences: Vec<String>, jwks_url: Url) -> Self {
        Self {
            allowed_audiences,
            jwks_cache: JwksCache::new(jwks_url),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(allowed_audiences: Vec<String>, jwks_url: Url) -> Self {
        Self::with_jwks_url(allowed_audiences, jwks_url)
    }
}

impl AppleJwksIdTokenVerifier {
    /// Verifies a Sign in with Apple ID token.
    ///
    /// The identity is keyed on `sub` only: Apple users may hide or relay their
    /// email, so email claims are ignored. A nonce is not required.
    pub async fn verify(
        &self,
        id_token: &str,
    ) -> Result<VerifiedAppleIdentity, AppleIdTokenVerificationError> {
        if id_token.trim().is_empty() || id_token.len() > MAX_ID_TOKEN_BYTES {
            return Err(AppleIdTokenVerificationError::Invalid);
        }

        let header = decode_header(id_token).map_err(|_| AppleIdTokenVerificationError::Invalid)?;
        if header.alg != Algorithm::RS256 {
            return Err(AppleIdTokenVerificationError::Invalid);
        }
        let kid = header.kid.ok_or(AppleIdTokenVerificationError::Invalid)?;

        let jwk = self
            .jwks_cache
            .find_key(&kid)
            .await
            .map_err(|_| AppleIdTokenVerificationError::DependencyUnavailable)?
            .ok_or(AppleIdTokenVerificationError::Invalid)?;

        let decoding_key = DecodingKey::from_jwk(&jwk)
            .map_err(|_| AppleIdTokenVerificationError::DependencyUnavailable)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&self.allowed_audiences);
        validation.set_issuer(&[APPLE_ISSUER]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = CLOCK_SKEW_LEEWAY_SECS;

        let token = decode::<AppleClaims>(id_token, &decoding_key, &validation)
            .map_err(|_| AppleIdTokenVerificationError::Invalid)?;

        if token.claims.sub.trim().is_empty() {
            return Err(AppleIdTokenVerificationError::Invalid);
        }
        if token.claims.iat > now_unix_secs() + CLOCK_SKEW_LEEWAY_SECS {
            return Err(AppleIdTokenVerificationError::Invalid);
        }

        let AppleClaims { iss, sub, .. } = token.claims;
        Ok(VerifiedAppleIdentity {
            issuer: iss,
            subject: sub,
        })
    }
}

/// `aud` is read as a single string: Apple always issues one audience, so a
/// multi-valued `aud` is rejected rather than matched on any member.
#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct AppleClaims {
    iss: String,
    sub: String,
    aud: String,
    iat: u64,
}

fn now_unix_secs() -> u64 {
    jsonwebtoken::get_current_timestamp()
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    use base64::prelude::{BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD, Engine as _};
    use jsonwebtoken::{EncodingKey, Header, encode, jwk::Jwk};
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    pub(crate) const TEST_APPLE_AUDIENCE: &str = "app.pubky.passport.test";
    pub(crate) const TEST_APPLE_SECOND_AUDIENCE: &str = "app.pubky.passport.test.ios";
    pub(crate) const TEST_KID: &str = "apple-test-kid";
    const TEST_RSA_PRIVATE_KEY_DER_BASE64: &str = "MIIEowIBAAKCAQEAyeEXgS0VqHMa2Ay0+dsGaGTTJDgv4RLAfTeqPpV8ot/00n9+5MwA31envjOM/NJlhVj6WBJINMTGbwKOxxSEso8XnsxNN/8iR9TndVnsFn894Lai4ljFImi60PVafuoJlILSFVmgIr2Yu0TGFP9jVhBHNrYu+jna3uhQqwEagtjeqRIisYsQevFz8TwfSbsLr5jFh4zEJhRrtfHxyCVqNdti776Ycrm+EDkzwEneKCHYMnHIuCbUfv+Sz+rAbwg7GUl+oPkfx1yeaYfaxN/W7b5gXkod7evUyvGnkMPCB5pIiP6FP8JGJKB8uGW8TRsI0kIJmgmydhKKqkGI8DXj1QIDAQABAoIBAApr5Mc7Lmrfj6Cx6JEcuz7ItFT305JBDLp0v69IZJfnNh+8YNlrJeu2/jFQPvR5i1pwEUhaSpZIHDL0hQNbmKK7xqKLdbxbdDJqGA1o+B+MhQNkhOcTYbsuKzVd8m8qd2vOZGL2w/tXIfB06JbX55G0oxXSH321CTVISoscB6QpJLzMEMNQiSVHbnm4LLISeRkw1agYdmdkGxGEAj7fQis8lhLviEZTSzDPk2DjhPAxFgAk9xaJjX5Od6G66lbK0bW7BBcpTqpoQ8hY7a0S7TRXSX3oHm2SOMBWXu2yOyjszfGKR0cuKaoZ3VYaSjx7JyKfjcdcKUzVE4NwhBvi5EECgYEA6BJn+LqUelRChjGGzAI9qdrns5PydFDmEkGnMCt7FUFLrwKhv90fkeXe5zJqa10cSfaps/qB/6zomXtVJatPacPDopjADSILUUnWmdeRGJWueAha29tS0oq3Vdg0+NjTwmfJqC1qgXtIboI+VWGz4u1YTq2zkRMiwoPpDqMt36ECgYEA3rG+adNZ4SpcKmlCDOa4zprZFdDjFHtPolbdF658K7qUa9ZkMseZ+21GMdVMnRnkVceh85w88svP7PgGfAb9+pHdPsdu2Twa/pvmOFAsdQakqOXi0rYx34Oz1fJuqarf+9tY8jLvcgLiFH6Y4SeOK/O3mCwq5hm5T9+EdnytZ7UCgYAEBrW5Efubd7TDwT42cd+B/+M/MVTmgbTMIkl1Q3vaLYzTl3cY7gwrKtLILg68mBvyiI3/KlxxBB3w87vRcHxrOFM/kejbEXkiyuYnOx8wTwB84qA46kAwmOl/WsXof1KFL+nGuir73hiumiHct1o4mKeMjeTsomla+JDliR01oQKBgA9uJ3BzuY2Vfs9dfbshHXsLVbFiSOm5ZZotBKLIb0gkZHZYY5b1J6E5QBzh164nBiA/deRIVhzIrH6R7PkZDu4Wv0rDTNX2iTRNHxWV0I7HSA9/5kiNYUVIfeR8KNjOwzhTSYV2rAqtUnbzEqq/L+WxvG3p2YsaCzISnb4MA4PlAoGBAKhLWgf+YvGDKa4UTIfWr0Mx0AqSq7M30wG3pV5M/w4fwMN5GQnM7MaOPiWQzwQu493ayYmLcU+Cu+r9JiL+aAulYQeUpwIGTkDER2+GGImWWre/71vCYQiWm0AnMZ0337tDffu751a4kh6Pva20DTb1VLTn+Utcw9Myz/Q+vjc9";

    pub(crate) async fn jwks_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/keys"))
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
        format!("{}/auth/keys", server.uri()).parse().unwrap()
    }

    pub(crate) fn test_jwk() -> Jwk {
        let encoding_key = EncodingKey::from_rsa_der(&test_private_key_der());
        let mut jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::RS256)
            .expect("test JWK should be built from RSA key");
        jwk.common.key_id = Some(TEST_KID.to_string());
        jwk
    }

    /// Claims shaped like a real Sign in with Apple ID token for `subject`.
    pub(crate) fn valid_claims(subject: &str) -> Value {
        let now = now_unix_secs();
        json!({
            "iss": APPLE_ISSUER,
            "aud": TEST_APPLE_AUDIENCE,
            "exp": now + 600,
            "iat": now,
            "sub": subject,
            "c_hash": "test-c-hash",
            "email": "relay@privaterelay.appleid.com",
            "email_verified": true,
            "is_private_email": true,
            "auth_time": now,
            "nonce_supported": true,
        })
    }

    pub(crate) fn valid_token(subject: &str) -> String {
        sign_claims(&valid_claims(subject))
    }

    pub(crate) fn token_with(subject: &str, overrides: Value) -> String {
        let mut claims = valid_claims(subject);
        for (key, value) in overrides
            .as_object()
            .expect("overrides should be an object")
        {
            claims[key] = value.clone();
        }
        sign_claims(&claims)
    }

    pub(crate) fn token_without_claim(subject: &str, claim: &str) -> String {
        let mut claims = valid_claims(subject);
        claims
            .as_object_mut()
            .expect("claims should be an object")
            .remove(claim);
        sign_claims(&claims)
    }

    pub(crate) fn wrong_audience_token() -> String {
        token_with("apple-subject", json!({ "aud": "other.services.id" }))
    }

    pub(crate) fn token_with_kid(subject: &str, kid: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        encode(&header, &valid_claims(subject), &test_encoding_key())
            .expect("test token should encode")
    }

    /// An HS256 token "signed" with the RSA public key bytes as HMAC secret
    /// (the classic algorithm-confusion attack).
    pub(crate) fn hs256_token(subject: &str) -> String {
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some(TEST_KID.to_string());
        let public_key_secret = test_jwk_modulus();
        encode(
            &header,
            &valid_claims(subject),
            &EncodingKey::from_secret(public_key_secret.as_bytes()),
        )
        .expect("test token should encode")
    }

    /// An unsigned token with `alg: none`.
    pub(crate) fn alg_none_token(subject: &str) -> String {
        let header = json!({ "alg": "none", "typ": "JWT", "kid": TEST_KID });
        format!(
            "{}.{}.",
            BASE64_URL_SAFE_NO_PAD.encode(header.to_string()),
            BASE64_URL_SAFE_NO_PAD.encode(valid_claims(subject).to_string()),
        )
    }

    /// A correctly signed token whose payload was swapped for another subject afterwards.
    pub(crate) fn tampered_payload_token(subject: &str, forged_subject: &str) -> String {
        let token = valid_token(subject);
        let mut parts = token.split('.');
        let header = parts.next().expect("token header");
        let _payload = parts.next().expect("token payload");
        let signature = parts.next().expect("token signature");
        let forged_payload =
            BASE64_URL_SAFE_NO_PAD.encode(valid_claims(forged_subject).to_string());
        format!("{header}.{forged_payload}.{signature}")
    }

    pub(crate) fn sign_claims(claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        encode(&header, claims, &test_encoding_key()).expect("test token should encode")
    }

    fn test_encoding_key() -> EncodingKey {
        EncodingKey::from_rsa_der(&test_private_key_der())
    }

    fn test_jwk_modulus() -> String {
        match test_jwk().algorithm {
            jsonwebtoken::jwk::AlgorithmParameters::RSA(rsa) => rsa.n,
            _ => unreachable!("test JWK is an RSA key"),
        }
    }

    fn test_private_key_der() -> Vec<u8> {
        BASE64_STANDARD
            .decode(TEST_RSA_PRIVATE_KEY_DER_BASE64)
            .expect("test RSA key should decode")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{Duration, Instant};

    use jsonwebtoken::jwk::JwkSet;
    use serde_json::json;
    use test_support::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use crate::shared::jwks_cache::MIN_FORCED_JWKS_REFRESH_INTERVAL;

    fn verifier_for_server(server: &MockServer) -> AppleJwksIdTokenVerifier {
        AppleJwksIdTokenVerifier::for_test(
            vec![
                TEST_APPLE_AUDIENCE.to_string(),
                TEST_APPLE_SECOND_AUDIENCE.to_string(),
            ],
            jwks_url(server),
        )
    }

    async fn assert_invalid(token: &str) {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);

        let error = verifier.verify(token).await.unwrap_err();

        assert!(
            matches!(error, AppleIdTokenVerificationError::Invalid),
            "expected Invalid, got {error:?}"
        );
    }

    #[tokio::test]
    async fn test_verifies_valid_apple_token() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);

        let identity = verifier
            .verify(&valid_token("apple-subject"))
            .await
            .unwrap();

        assert_eq!(identity.issuer, "https://appleid.apple.com");
        assert_eq!(identity.subject, "apple-subject");
    }

    #[tokio::test]
    async fn test_accepts_any_configured_audience() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = token_with(
            "apple-subject",
            json!({ "aud": TEST_APPLE_SECOND_AUDIENCE }),
        );

        let identity = verifier.verify(&token).await.unwrap();

        assert_eq!(identity.subject, "apple-subject");
    }

    #[tokio::test]
    async fn test_accepts_token_without_email_or_nonce() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = sign_minimal_claims("apple-subject");

        let identity = verifier.verify(&token).await.unwrap();

        assert_eq!(identity.subject, "apple-subject");
    }

    #[tokio::test]
    async fn test_accepts_iat_within_clock_skew() {
        let server = jwks_server().await;
        let verifier = verifier_for_server(&server);
        let token = token_with(
            "apple-subject",
            json!({ "iat": now_unix_secs() + CLOCK_SKEW_LEEWAY_SECS / 2 }),
        );

        let identity = verifier.verify(&token).await.unwrap();

        assert_eq!(identity.subject, "apple-subject");
    }

    #[tokio::test]
    async fn test_rejects_wrong_audience() {
        assert_invalid(&wrong_audience_token()).await;
    }

    #[tokio::test]
    async fn test_rejects_wrong_issuer() {
        assert_invalid(&token_with(
            "apple-subject",
            json!({ "iss": "https://evil.example" }),
        ))
        .await;
    }

    #[tokio::test]
    async fn test_rejects_bare_apple_issuer() {
        assert_invalid(&token_with(
            "apple-subject",
            json!({ "iss": "appleid.apple.com" }),
        ))
        .await;
    }

    #[tokio::test]
    async fn test_rejects_google_issuer() {
        assert_invalid(&token_with(
            "apple-subject",
            json!({ "iss": "https://accounts.google.com" }),
        ))
        .await;
    }

    #[tokio::test]
    async fn test_rejects_expired_token() {
        let now = now_unix_secs();
        assert_invalid(&token_with(
            "apple-subject",
            json!({ "iat": now - 7200, "exp": now - 3600 }),
        ))
        .await;
    }

    #[tokio::test]
    async fn test_rejects_missing_exp() {
        assert_invalid(&token_without_claim("apple-subject", "exp")).await;
    }

    #[tokio::test]
    async fn test_rejects_future_iat() {
        let now = now_unix_secs();
        assert_invalid(&token_with(
            "apple-subject",
            json!({ "iat": now + 3600, "exp": now + 7200 }),
        ))
        .await;
    }

    #[tokio::test]
    async fn test_rejects_missing_iat() {
        assert_invalid(&token_without_claim("apple-subject", "iat")).await;
    }

    #[tokio::test]
    async fn test_rejects_missing_audience() {
        assert_invalid(&token_without_claim("apple-subject", "aud")).await;
    }

    #[tokio::test]
    async fn test_rejects_multi_valued_audience() {
        assert_invalid(&token_with(
            "apple-subject",
            json!({ "aud": [TEST_APPLE_AUDIENCE, "other.services.id"] }),
        ))
        .await;
    }

    #[tokio::test]
    async fn test_rejects_missing_subject() {
        assert_invalid(&token_without_claim("apple-subject", "sub")).await;
    }

    #[tokio::test]
    async fn test_rejects_empty_subject() {
        assert_invalid(&valid_token("")).await;
        assert_invalid(&valid_token("   ")).await;
    }

    #[tokio::test]
    async fn test_rejects_bad_signature() {
        assert_invalid(&tampered_payload_token("apple-subject", "other-subject")).await;
    }

    #[tokio::test]
    async fn test_rejects_unknown_kid() {
        assert_invalid(&token_with_kid("apple-subject", "unknown-kid")).await;
    }

    #[tokio::test]
    async fn test_rejects_alg_none() {
        assert_invalid(&alg_none_token("apple-subject")).await;
    }

    #[tokio::test]
    async fn test_rejects_hs256() {
        assert_invalid(&hs256_token("apple-subject")).await;
    }

    #[tokio::test]
    async fn test_rejects_empty_and_oversized_tokens() {
        assert_invalid("").await;
        assert_invalid("   ").await;
        assert_invalid("not-a-jwt").await;
        assert_invalid(&"a".repeat(MAX_ID_TOKEN_BYTES + 1)).await;
    }

    #[tokio::test]
    async fn test_maps_jwks_fetch_failure_to_dependency_unavailable() {
        let server = MockServer::start().await;
        let verifier = verifier_for_server(&server);

        let error = verifier
            .verify(&valid_token("apple-subject"))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AppleIdTokenVerificationError::DependencyUnavailable
        ));
    }

    #[tokio::test]
    async fn test_unknown_kid_with_failed_refresh_is_dependency_unavailable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/keys"))
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
            .verify(&valid_token("apple-subject"))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AppleIdTokenVerificationError::DependencyUnavailable
        ));
    }

    fn sign_minimal_claims(subject: &str) -> String {
        let now = now_unix_secs();
        sign_claims(&json!({
            "iss": APPLE_ISSUER,
            "aud": TEST_APPLE_AUDIENCE,
            "exp": now + 600,
            "iat": now,
            "sub": subject,
        }))
    }
}
