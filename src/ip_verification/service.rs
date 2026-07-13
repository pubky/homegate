use std::net::IpAddr;

use crate::infrastructure::config::IpVerificationConfig;
use crate::infrastructure::sql::SqlDb;
use crate::shared::HasherArgon2id;
use crate::shared::HomeserverAdminAPI;
use crate::shared::{LimitEnforcement, RateLimitedSignupIssuer, VerificationTable};

use super::error::IpVerificationError;
use super::types::IpVerificationResponse;

const IP_VERIFICATIONS_TABLE: VerificationTable = VerificationTable {
    name: "ip_verifications",
    hash_column: "ip_address_hash",
};

#[derive(Clone, Debug)]
pub struct IpVerificationService {
    signup_issuer: RateLimitedSignupIssuer,
    hasher_argon2id: HasherArgon2id,
    limit_whitelist: Vec<IpAddr>,
}

impl IpVerificationService {
    pub fn new(
        db: SqlDb,
        homeserver_admin_api: HomeserverAdminAPI,
        config: &IpVerificationConfig,
        hasher: HasherArgon2id,
    ) -> Self {
        if !config.limit_whitelist.is_empty() {
            tracing::info!(
                "IP verification limit whitelist: {:?}",
                config.limit_whitelist
            );
        }

        Self {
            signup_issuer: RateLimitedSignupIssuer::new(
                db,
                homeserver_admin_api,
                IP_VERIFICATIONS_TABLE,
                config.max_verifications_per_week,
                config.max_verifications_per_year,
                config.signup_quota.clone(),
            ),
            hasher_argon2id: hasher,
            limit_whitelist: config.limit_whitelist.clone(),
        }
    }

    #[cfg(test)]
    pub fn set_limit_whitelist(&mut self, whitelist: Vec<IpAddr>) {
        self.limit_whitelist = whitelist;
    }

    pub async fn verify(
        &self,
        ip_address: IpAddr,
    ) -> Result<IpVerificationResponse, IpVerificationError> {
        let enforcement = if self.limit_whitelist.contains(&ip_address) {
            LimitEnforcement::Bypass
        } else {
            LimitEnforcement::Enforce
        };
        let ip_hash = self.hasher_argon2id.hash(&ip_address.to_string());

        let issued = self.signup_issuer.issue(&ip_hash, enforcement).await?;

        Ok(IpVerificationResponse {
            signup_code: issued.signup_code,
            homeserver_pubky: issued.homeserver_pubky,
        })
    }
}
