use argon2::{
    Algorithm, Argon2, ParamsBuilder, PasswordHasher, Version, password_hash::SaltString,
};
use std::fs;
use std::path::PathBuf;

/// Hashes identifiers using Argon2id with a global pepper for rainbow table resistance.
///
/// This hasher was primarily built for phone numbers (SMS verification), whose
/// small preimage space makes them cheap to brute-force from a leaked database.
/// Other verification providers reuse it for their identifiers (IP addresses,
/// Google `iss`+`sub` claims) for consistency and simplicity: one pepper file,
/// one set of tuned parameters, and uniformly non-reversible rate-limit keys —
/// even where the preimage space would not strictly demand a hash this slow.
///
/// Choice of hash function:
///     The configured argon2id params use a substantial amount of RAM and > 100ms to compute.
///     If our db were to leak then this quality increases the time and resources required for an attacker to find the hash pre-images (user's phone number in this case).
///
/// Note: This hash function intentionally takes > 100ms to produce its digest.
#[derive(Clone, Debug)]
pub struct HasherArgon2id {
    pepper: Pepper,
    argon2: Argon2<'static>,
}

impl HasherArgon2id {
    pub fn new(pepper_path: PathBuf) -> Self {
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
        let pepper = Pepper::read_or_generate(pepper_path);
        Self { pepper, argon2 }
    }

    /// Hashes a string using Argon2id with a deterministic salt.
    pub fn hash(&self, preimage: &str) -> String {
        // Derive deterministic salt from pepper using Blake3
        let salt_bytes = blake3::hash(self.pepper.as_bytes());
        let salt = SaltString::encode_b64(&salt_bytes.as_bytes()[..16])
            .expect("Salt encoding should never fail with valid Blake3 output");

        let hash = self
            .argon2
            .hash_password(preimage.as_bytes(), &salt)
            .expect("Argon2 hashing should never fail with valid parameters and salt");

        let hash_output = hash
            .hash
            .expect("Argon2 hash output should always be present");

        hex::encode(hash_output.as_bytes())
    }
}

/// Pepper is used a bit like a salt for hashing identifiers (phone numbers, IPs, ...) in the db.
/// The difference is that the same value is used for all identifier instances, rather than a new salt per identifier.
/// We store the generated value in ~/.homegate/pepper.txt for convenience
#[derive(Clone, Debug)]
struct Pepper(String);

impl Pepper {
    fn read_or_generate(file_path: PathBuf) -> Self {
        if file_path.exists() {
            let content = fs::read_to_string(&file_path).expect("Failed to read pepper file");
            let content = content.trim();
            Self::validate_pepper_value(content);
            return Self(content.to_string());
        }

        // If not exist then generate new pepper and store
        let pepper = Self::generate_pepper_value();

        if let Some(dir_path) = file_path.parent() {
            fs::create_dir_all(dir_path).expect("Creating pepper directory should not fail");
        }
        fs::write(&file_path, pepper.as_bytes()).expect("Failed to write pepper file");

        tracing::info!("Generated new pepper at: {}", file_path.display());
        Self(pepper)
    }

    fn generate_pepper_value() -> String {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("Randomness generation should not fail");
        hex::encode(bytes)
    }

    fn validate_pepper_value(content: &str) {
        (content.len() == 64 && content.chars().all(|c| c.is_ascii_hexdigit()))
            .then_some(())
            .ok_or(())
            .expect("Pepper file should contain a 64 character hex string");
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hasher_init_creates_new_pepper() {
        let temp_dir = TempDir::new().unwrap();
        let pepper_file = temp_dir.path().join("pepper.txt");

        // Initialize hasher with custom pepper path
        let hasher = HasherArgon2id::new(pepper_file.clone());

        // Verify pepper file was created
        assert!(pepper_file.exists(), "Pepper file should be created");

        // Verify pepper content is valid (64 char hex string)
        let pepper_content = fs::read_to_string(&pepper_file).unwrap();
        let pepper_content = pepper_content.trim();
        assert_eq!(pepper_content.len(), 64, "Pepper should be 64 characters");
        assert!(
            pepper_content.chars().all(|c| c.is_ascii_hexdigit()),
            "Pepper should be hex string"
        );

        // Verify hasher is functional
        let hash = hasher.hash("+1234567890");
        assert!(!hash.is_empty(), "Should produce a hash");
    }

    #[test]
    fn test_hasher_init_loads_existing_pepper() {
        let temp_dir = TempDir::new().unwrap();
        let pepper_file = temp_dir.path().join("pepper.txt");

        // Write a known pepper
        let known_pepper = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        fs::write(&pepper_file, known_pepper).unwrap();

        // Initialize hasher - should load existing pepper
        let hasher1 = HasherArgon2id::new(pepper_file.clone());
        let hash1 = hasher1.hash("+1234567890");

        // Initialize another hasher - should load the same pepper
        let hasher2 = HasherArgon2id::new(pepper_file.clone());
        let hash2 = hasher2.hash("+1234567890");

        // Both hashers should produce the same hash since they use the same pepper
        assert_eq!(
            hash1, hash2,
            "Both hashers should produce identical hashes with same pepper"
        );

        // Verify the pepper file content hasn't changed
        let pepper_content = fs::read_to_string(&pepper_file).unwrap();
        assert_eq!(
            pepper_content.trim(),
            known_pepper,
            "Pepper file should not be modified"
        );
    }
}
