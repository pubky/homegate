use crate::infrastructure::sql::{SqlDb, UnifiedExecutor};
use crate::invite::error::InviteError;
use crate::invite::repository::InviteRepository;
use crate::invite::types::{InviteRequest, InviteResponse};
use crate::shared::HomeserverAdminAPI;
use pubky_common::crypto::PublicKey;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The pubky path where the proof hash is expected to be stored.
const PROOF_PATH: &str = "homegate/proof";

/// Shape of the JSON returned by the homeserver when reading a pubky file.
#[derive(Deserialize)]
struct PubkyFileResponse {
    value: String,
}

#[derive(Clone, Debug)]
pub struct InviteService {
    homeserver_admin_api: HomeserverAdminAPI,
    max_per_week: u32,
    max_per_year: u32,
}

impl InviteService {
    pub fn new(
        homeserver_admin_api: HomeserverAdminAPI,
        max_per_week: u32,
        max_per_year: u32,
    ) -> Self {
        Self {
            homeserver_admin_api,
            max_per_week,
            max_per_year,
        }
    }

    pub async fn invite(
        &self,
        db: &SqlDb,
        request: InviteRequest,
    ) -> Result<InviteResponse, InviteError> {
        // 1. Validate pubkey
        let public_key =
            PublicKey::try_from(request.pubkey.as_str()).map_err(|_| InviteError::InvalidPubkey)?;
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

        // 4. Check weekly limit
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

        // 5. Check annual limit
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

        // 6. Generate signup token from homeserver
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

        // 7. Insert unclaimed record
        InviteRepository::insert_unclaimed(&mut executor, &pubkey_z32, &proof_hash, &signup_code)
            .await?;

        drop(executor);
        tx.commit().await?;

        // 8. Return response
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
            .map_err(|_e| InviteError::HomeserverUnavailable)?;

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
