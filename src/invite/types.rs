use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteRequest {
    pub pubkey: String,
    pub hash_proof_preimage: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteResponse {
    pub signup_code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteStatus {
    Unclaimed,
    Claimed,
    Failed,
}

impl InviteStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unclaimed => "UNCLAIMED",
            Self::Claimed => "CLAIMED",
            Self::Failed => "FAILED",
        }
    }
}
