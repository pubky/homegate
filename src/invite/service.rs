use crate::infrastructure::sql::{SqlDb, UnifiedExecutor};
use crate::invite::error::InviteError;
use crate::invite::repository::InviteRepository;
use crate::invite::types::{InviteRequest, InviteResponse};
use crate::shared::HomeserverAdminAPI;
use pubky::Pubky;
use pubky_common::crypto::PublicKey;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The pubky path where the proof hash is expected to be stored.
const PROOF_PATH: &str = "pub/pubky.app/homegate/proof";

/// The pubky path to the user's posts directory.
const POSTS_PATH: &str = "pub/pubky.app/posts/";

/// Shape of the JSON returned by the homeserver when reading a pubky file.
#[derive(Deserialize)]
struct PubkyFileResponse {
    value: String,
}

#[derive(Clone, Debug)]
pub struct InviteService {
    homeserver_admin_api: HomeserverAdminAPI,
    pubky: Pubky,
    max_per_week: u32,
    max_per_year: u32,
    min_posts: u16,
}

impl InviteService {
    pub fn new(
        homeserver_admin_api: HomeserverAdminAPI,
        pubky: Pubky,
        max_per_week: u32,
        max_per_year: u32,
        min_posts: u16,
    ) -> Self {
        Self {
            homeserver_admin_api,
            pubky,
            max_per_week,
            max_per_year,
            min_posts,
        }
    }

    pub async fn invite(
        &self,
        db: &SqlDb,
        request: InviteRequest,
    ) -> Result<InviteResponse, InviteError> {
        // 1. Validate pubkey
        let public_key =
            PublicKey::try_from(request.pubky.as_str()).map_err(|_| InviteError::InvalidPubkey)?;
        let pubkey_z32 = public_key.z32();

        // 2. Verify proof: hash the preimage and compare with the file at /pub/homegate/proof
        let proof_hash = hex::encode(Sha256::digest(request.hash_proof_preimage.as_bytes()));
        self.verify_proof(&pubkey_z32, &proof_hash).await?;

        let mut tx = db.pool().begin().await?;
        let mut executor = UnifiedExecutor::from_tx(&mut tx);

        // 3. Check for an existing unclaimed token
        if let Some(signup_code) =
            InviteRepository::find_unclaimed_signup_code(&mut executor, &pubkey_z32).await?
        {
            // Verify with the homeserver whether it's actually been claimed
            let claimed = self
                .homeserver_admin_api
                .is_signup_token_claimed(&signup_code)
                .await
                .map_err(|_e| InviteError::HomeserverUnavailable)?;

            if claimed {
                // Mark as claimed locally and continue to generate a new token
                InviteRepository::mark_claimed(&mut executor, &signup_code).await?;
            } else {
                // No DB changes needed, rollback is fine
                return Ok(InviteResponse { signup_code });
            }
        }

        // 4. Check minimum post count via pubky SDK
        if self.min_posts > 0 {
            let posts = match self
                .pubky
                .public_storage()
                .list(format!("pubky:{}/{}", pubkey_z32, POSTS_PATH))
                .map_err(|e| {
                    tracing::error!("Pubky list build failed: {:?}", e);
                    InviteError::HomeserverUnavailable
                })?
                .limit(self.min_posts)
                .shallow(true)
                .send()
                .await
            {
                Ok(posts) => posts,
                Err(e) if e.to_string().contains("404") => {
                    // No posts directory means zero posts
                    vec![]
                }
                Err(e) => {
                    tracing::error!("Pubky list posts failed: {:?}", e);
                    return Err(InviteError::HomeserverUnavailable);
                }
            };
            if (posts.len() as u16) < self.min_posts {
                tracing::warn!(
                    pubkey = %pubkey_z32,
                    post_count = posts.len(),
                    min_posts = self.min_posts,
                    "Insufficient posts for invite"
                );
                return Err(InviteError::InsufficientPosts);
            }
        }

        // 5. Check weekly limit
        let weekly_count =
            InviteRepository::count_claimed_in_last_days(&mut executor, &pubkey_z32, 7).await?;
        if weekly_count >= self.max_per_week as i64 {
            tracing::warn!(
                pubkey = %pubkey_z32,
                weekly_count = weekly_count,
                weekly_limit = self.max_per_week,
                "Weekly invite limit exceeded"
            );
            return Err(InviteError::WeeklyLimitExceeded);
        }

        // 6. Check annual limit
        let annual_count =
            InviteRepository::count_claimed_in_last_days(&mut executor, &pubkey_z32, 365).await?;
        if annual_count >= self.max_per_year as i64 {
            tracing::warn!(
                pubkey = %pubkey_z32,
                annual_count = annual_count,
                annual_limit = self.max_per_year,
                "Annual invite limit exceeded"
            );
            return Err(InviteError::AnnualLimitExceeded);
        }

        // 7. Generate signup token from homeserver
        let signup_code = match self.homeserver_admin_api.generate_signup_token().await {
            Ok(code) => code,
            Err(_e) => {
                if let Err(e) = InviteRepository::insert_failed(
                    &mut executor,
                    &pubkey_z32,
                    &proof_hash,
                    "homeserver_signup_token_generation_failed",
                )
                .await
                {
                    tracing::error!("{}", e);
                }
                drop(executor);
                tx.commit().await?;
                return Err(InviteError::HomeserverUnavailable);
            }
        };

        // 8. Insert unclaimed record
        InviteRepository::insert_unclaimed(&mut executor, &pubkey_z32, &proof_hash, &signup_code)
            .await?;

        drop(executor);
        tx.commit().await?;

        // 9. Return response
        Ok(InviteResponse { signup_code })
    }

    /// Verify the proof by comparing the expected hash with the content
    /// stored at the inviter's pubky path /pub/homegate/proof.
    ///
    /// The homeserver returns the file content as JSON: `{ "value": "<hex>" }`.
    async fn verify_proof(&self, pubkey_z32: &str, expected_hash: &str) -> Result<(), InviteError> {
        let file_content = self
            .homeserver_admin_api
            .get_pubky_file(pubkey_z32, PROOF_PATH)
            .await
            .map_err(|e| {
                tracing::error!("Proof fetch failed: {:?}", e);
                InviteError::HomeserverUnavailable
            })?;

        let raw = file_content.ok_or(InviteError::ProofNotFound)?;

        let parsed: PubkyFileResponse =
            serde_json::from_str(&raw).map_err(|_| InviteError::ProofMismatch)?;
        let stored_hash = parsed.value.trim();

        if stored_hash != expected_hash {
            tracing::warn!(
                pubkey = %pubkey_z32,
                "Proof hash mismatch"
            );
            return Err(InviteError::ProofMismatch);
        }

        Ok(())
    }
}
