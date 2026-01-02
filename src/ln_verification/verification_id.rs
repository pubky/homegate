use std::{fmt::Display, str::FromStr};

use uuid::Uuid;

/// Error type for verification ID parse operations
#[derive(thiserror::Error, Debug)]
pub enum VerificationIdError {
    #[error("Invalid verification ID format: {0}")]
    InvalidFormat(String),
}

/// Verification ID for Lightning Network verifications.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VerificationId(Uuid);

impl VerificationId {
    #[cfg(test)]
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Display for VerificationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for VerificationId {
    type Err = VerificationIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid =
            Uuid::parse_str(s).map_err(|e| VerificationIdError::InvalidFormat(e.to_string()))?;
        Ok(Self(uuid))
    }
}

impl serde::Serialize for VerificationId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for VerificationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl sqlx::Type<sqlx::Postgres> for VerificationId {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        sqlx::postgres::PgTypeInfo::with_name("uuid")
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for VerificationId {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let uuid = <Uuid as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(Self(uuid))
    }
}
