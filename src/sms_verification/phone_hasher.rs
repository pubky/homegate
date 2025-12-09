use argon2::{
    Algorithm, Argon2, ParamsBuilder, PasswordHasher, Version, password_hash::SaltString,
};
use thiserror::Error;

/// Errors that can occur during phone number hashing
#[derive(Error, Debug)]
pub enum PhoneHasherError {
    #[error("Failed to encode salt: {0}")]
    SaltEncodingError(String),

    #[error("Failed to hash phone number: {0}")]
    HashingError(String),

    #[error("Hash output is missing from Argon2 result")]
    MissingHashOutput,
}

/// Hashes phone numbers using Argon2id with a global pepper for rainbow table resistance.
/// We do this as a mitigation against rainbow table attacks if our db were to leak
#[derive(Clone, Debug)]
pub struct PhoneHasher {
    pepper: String,
    argon2: Argon2<'static>,
}

impl PhoneHasher {
    /// Creates a new PhoneHasher with the given pepper.
    pub fn new(pepper: String) -> Self {
        // OWASP recommended parameters for Argon2id
        // Memory: 19456 KiB (19 MiB)
        // Iterations: 2
        // Parallelism: 1
        // Output: 32 bytes
        let params = ParamsBuilder::new()
            .m_cost(19456)
            .t_cost(2)
            .p_cost(1)
            .output_len(32)
            .build()
            .expect("Failed to build Argon2 params");

        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        Self { pepper, argon2 }
    }

    /// Hashes a phone number using Argon2id with a deterministic salt.
    /// Returns an error if hashing fails (extremely rare)
    pub fn hash_phone_number(&self, phone_number: &str) -> Result<String, PhoneHasherError> {
        // Derive deterministic salt from pepper using Blake3
        let salt_bytes = blake3::hash(self.pepper.as_bytes());
        let salt_b64 = base64::engine::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            salt_bytes.as_bytes(),
        );
        let salt = SaltString::encode_b64(&salt_b64.as_bytes()[..16])
            .map_err(|e| PhoneHasherError::SaltEncodingError(e.to_string()))?;

        let hash = self
            .argon2
            .hash_password(phone_number.as_bytes(), &salt)
            .map_err(|e| PhoneHasherError::HashingError(e.to_string()))?;

        let hash_output = hash.hash.ok_or(PhoneHasherError::MissingHashOutput)?;

        Ok(hex::encode(hash_output.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_hashing() {
        let hasher = PhoneHasher::new("test-pepper".to_string());
        let phone = "+1234567890";

        let hash1 = hasher.hash_phone_number(phone).unwrap();
        let hash2 = hasher.hash_phone_number(phone).unwrap();

        // Same phone number should produce same hash
        assert_eq!(hash1, hash2);
        // Hash should be 64 hex characters (32 bytes)
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_different_numbers_produce_different_hashes() {
        let hasher = PhoneHasher::new("test-pepper".to_string());

        let hash1 = hasher.hash_phone_number("+1234567890").unwrap();
        let hash2 = hasher.hash_phone_number("+0987654321").unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_different_peppers_produce_different_hashes() {
        let hasher1 = PhoneHasher::new("pepper1".to_string());
        let hasher2 = PhoneHasher::new("pepper2".to_string());
        let phone = "+1234567890";

        let hash1 = hasher1.hash_phone_number(phone).unwrap();
        let hash2 = hasher2.hash_phone_number(phone).unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_format() {
        let hasher = PhoneHasher::new("test-pepper".to_string());
        let hash = hasher.hash_phone_number("+1234567890").unwrap();

        // Should be valid hex
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        // Should be 64 characters (32 bytes in hex)
        assert_eq!(hash.len(), 64);
    }
}
