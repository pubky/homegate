use argon2::{
    Algorithm, Argon2, ParamsBuilder, PasswordHasher, Version, password_hash::SaltString,
};

/// Hashes phone numbers using Argon2id with a global pepper for rainbow table resistance.
///
/// Choice of hash function:
///     The configured argon2id params use a substantial amount of RAM and > 100ms to compute.
///     If our db were to leak then this quality increases the time and resources required for an attacker to find the hash pre-images (user's phone number in this case).
///
/// Note: This hash function intentionally takes > 100ms to produce its hash value.
#[derive(Clone, Debug)]
pub struct HasherArgon2id {
    pepper: String,
    argon2: Argon2<'static>,
}

impl HasherArgon2id {
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

    /// Hashes a string using Argon2id with a deterministic salt.
    pub fn hash_phone_number(&self, preimage: &str) -> String {
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
