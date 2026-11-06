use argon2::{
    Algorithm, Argon2, ParamsBuilder, PasswordHasher, Version, password_hash::SaltString,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

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
/// [`HasherArgon2id::hash`] therefore runs it on tokio's blocking thread pool,
/// so it never stalls the async workers that serve every other request, and
/// caps how many hashes run at once (one per CPU unless configured). When every slot is taken it fails fast
/// with [`HasherBusy`] instead of queueing work without bound.
#[derive(Clone, Debug)]
pub struct HasherArgon2id {
    pepper: Pepper,
    argon2: Argon2<'static>,
    /// Shared by all clones, so the cap applies process-wide.
    hash_slots: Arc<Semaphore>,
}

/// Every hashing slot is in use. Callers should answer "try again later"
/// (HTTP 503) rather than wait.
#[derive(thiserror::Error, Debug)]
#[error("all identifier hashing slots are busy")]
pub struct HasherBusy;

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
        Self {
            pepper,
            argon2,
            hash_slots: Arc::new(Semaphore::new(default_max_concurrent_hashes())),
        }
    }

    /// Hashes a string using Argon2id with a deterministic salt.
    ///
    /// Runs on the blocking thread pool. Returns [`HasherBusy`] immediately if
    /// the maximum number of concurrent hashes is already running.
    pub async fn hash(&self, preimage: &str) -> Result<String, HasherBusy> {
        let slot = Arc::clone(&self.hash_slots)
            .try_acquire_owned()
            .map_err(|_| HasherBusy)?;
        let hasher = self.clone();
        let preimage = preimage.to_owned();
        // The slot moves into the blocking task, so it is released only when
        // the hash has actually finished (or panicked), even if the caller's
        // future is dropped first (client disconnect, request timeout).
        let task = tokio::task::spawn_blocking(move || {
            let _slot = slot;
            hasher.hash_blocking(&preimage)
        });
        match task.await {
            Ok(hash) => Ok(hash),
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            // Only happens while the runtime is shutting down.
            Err(_) => Err(HasherBusy),
        }
    }

    fn hash_blocking(&self, preimage: &str) -> String {
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

    /// Replaces the default cap on concurrent hashes (one per CPU).
    pub fn with_max_concurrent_hashes(mut self, max: usize) -> Self {
        assert!(max > 0, "at least one concurrent hash must be allowed");
        self.hash_slots = Arc::new(Semaphore::new(max));
        self
    }

    /// Takes every free hashing slot until the returned permit is dropped, so
    /// tests can put the hasher in the "busy" state deterministically.
    #[cfg(test)]
    pub(crate) fn occupy_all_slots(&self) -> tokio::sync::OwnedSemaphorePermit {
        let free = self.hash_slots.available_permits() as u32;
        Arc::clone(&self.hash_slots)
            .try_acquire_many_owned(free)
            .expect("hash slots are never closed")
    }
}

/// One hash per CPU. Hashing is CPU-bound, so more slots would not add
/// throughput, only memory (19 MiB per running hash).
fn default_max_concurrent_hashes() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
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
    use std::time::Duration;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_hasher_init_creates_new_pepper() {
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
        let hash = hasher.hash("+1234567890").await.unwrap();
        assert!(!hash.is_empty(), "Should produce a hash");
    }

    #[tokio::test]
    async fn test_hasher_init_loads_existing_pepper() {
        let temp_dir = TempDir::new().unwrap();
        let pepper_file = temp_dir.path().join("pepper.txt");

        // Write a known pepper
        let known_pepper = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        fs::write(&pepper_file, known_pepper).unwrap();

        // Initialize hasher - should load existing pepper
        let hasher1 = HasherArgon2id::new(pepper_file.clone());
        let hash1 = hasher1.hash("+1234567890").await.unwrap();

        // Initialize another hasher - should load the same pepper
        let hasher2 = HasherArgon2id::new(pepper_file.clone());
        let hash2 = hasher2.hash("+1234567890").await.unwrap();

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

    const KNOWN_PEPPER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn hasher_with_known_pepper(dir: &TempDir) -> HasherArgon2id {
        let pepper_file = dir.path().join("pepper.txt");
        fs::write(&pepper_file, KNOWN_PEPPER).unwrap();
        HasherArgon2id::new(pepper_file)
    }

    /// Stored rate-limit keys must keep matching: these digests were produced
    /// by the previous, synchronous implementation for the same pepper.
    #[tokio::test]
    async fn test_hash_output_is_unchanged() {
        let temp_dir = TempDir::new().unwrap();
        let hasher = hasher_with_known_pepper(&temp_dir);

        let expected = [
            (
                "+1234567890",
                "86d291d8ce2d590fea59463d13a8ddd33842b7887ab84d88bc13e95e92c4b883",
            ),
            (
                "203.0.113.7",
                "ad55a7b0abeec2f878f466cff1961305a7cbdbe54529584b83b08114618c12c0",
            ),
            (
                "https://accounts.google.com\n1234567890",
                "370c5c4caed8c2a9db3c21687d50775730413f954dfe644d9a44cd70f6868b08",
            ),
        ];
        for (preimage, digest) in expected {
            assert_eq!(hasher.hash(preimage).await.unwrap(), digest, "{preimage:?}");
        }
    }

    #[test]
    fn test_cap_defaults_to_cpu_count_and_can_be_overridden() {
        let cpus = std::thread::available_parallelism().unwrap().get();
        let temp_dir = TempDir::new().unwrap();
        let hasher = hasher_with_known_pepper(&temp_dir);
        assert_eq!(hasher.hash_slots.available_permits(), cpus);

        let hasher = hasher.with_max_concurrent_hashes(3);
        assert_eq!(hasher.hash_slots.available_permits(), 3);
    }

    #[tokio::test]
    async fn test_busy_when_all_slots_taken() {
        let temp_dir = TempDir::new().unwrap();
        let hasher = hasher_with_known_pepper(&temp_dir).with_max_concurrent_hashes(2);

        let occupied = hasher.occupy_all_slots();
        let result = tokio::time::timeout(Duration::from_secs(1), hasher.hash("+1234567890"))
            .await
            .expect("a full hasher must answer immediately, not wait for a slot");
        assert!(matches!(result, Err(HasherBusy)));

        drop(occupied);
        assert!(hasher.hash("+1234567890").await.is_ok());
    }

    #[tokio::test]
    async fn test_slot_held_while_hashing_and_shared_by_clones() {
        let temp_dir = TempDir::new().unwrap();
        let hasher = hasher_with_known_pepper(&temp_dir).with_max_concurrent_hashes(1);
        let clone = hasher.clone();

        let mut in_flight = std::pin::pin!(hasher.hash("+1234567890"));
        assert!(futures_util::poll!(&mut in_flight).is_pending());
        assert_eq!(hasher.hash_slots.available_permits(), 0);
        assert!(matches!(clone.hash("+1987654321").await, Err(HasherBusy)));

        in_flight.await.unwrap();
        assert_eq!(hasher.hash_slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn test_slot_released_when_caller_is_dropped() {
        let temp_dir = TempDir::new().unwrap();
        let hasher = hasher_with_known_pepper(&temp_dir).with_max_concurrent_hashes(1);

        let mut in_flight = Box::pin(hasher.hash("+1234567890"));
        assert!(futures_util::poll!(&mut in_flight).is_pending());
        // Dropping the caller (client went away) leaves the blocking hash
        // running; its slot must come back once that hash is done.
        drop(in_flight);

        tokio::time::timeout(Duration::from_secs(10), async {
            while hasher.hash_slots.available_permits() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("slot was never released");
        assert!(hasher.hash("+1234567890").await.is_ok());
    }
}
