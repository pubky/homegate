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

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for VerificationId {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&Uuid as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&&self.0, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_uuid() {
        let valid_uuid = "550e8400-e29b-41d4-a716-446655440000";
        let verification_id = VerificationId::from_str(valid_uuid).unwrap();
        assert_eq!(verification_id.to_string(), valid_uuid);
    }

    #[test]
    fn test_valid_uuid_uppercase() {
        let uppercase_uuid = "550E8400-E29B-41D4-A716-446655440000";
        let verification_id = VerificationId::from_str(uppercase_uuid).unwrap();
        // UUIDs are normalized to lowercase
        assert_eq!(
            verification_id.to_string(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_invalid_format_not_uuid() {
        let invalid = "not-a-uuid";
        let result = VerificationId::from_str(invalid);
        assert!(result.is_err());
        match result.unwrap_err() {
            VerificationIdError::InvalidFormat(_) => {}
        }
    }

    #[test]
    fn test_invalid_format_empty() {
        let empty = "";
        let result = VerificationId::from_str(empty);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_format_wrong_length() {
        let wrong_length = "550e8400-e29b-41d4-a716";
        let result = VerificationId::from_str(wrong_length);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_format_invalid_hex() {
        // UUID parser actually accepts UUIDs without hyphens, so test invalid hex instead
        let invalid_hex = "550e8400-e29b-41d4-a716-44665544000g";
        let result = VerificationId::from_str(invalid_hex);
        assert!(result.is_err());
    }

    #[test]
    fn test_display_trait() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let verification_id = VerificationId::from_str(uuid_str).unwrap();
        assert_eq!(format!("{}", verification_id), uuid_str);
    }

    #[test]
    fn test_from_str_valid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let verification_id: VerificationId = uuid_str.parse().unwrap();
        assert_eq!(verification_id.to_string(), uuid_str);
    }

    #[test]
    fn test_from_str_invalid() {
        let invalid = "invalid-uuid-format";
        let result: Result<VerificationId, _> = invalid.parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let verification_id = VerificationId::from_str(uuid_str).unwrap();
        let serialized = serde_json::to_string(&verification_id).unwrap();
        assert_eq!(serialized, format!("\"{}\"", uuid_str));
    }

    #[test]
    fn test_deserialize_valid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let json = format!("\"{}\"", uuid_str);
        let verification_id: VerificationId = serde_json::from_str(&json).unwrap();
        assert_eq!(verification_id.to_string(), uuid_str);
    }

    #[test]
    fn test_deserialize_invalid_format() {
        let json = "\"not-a-uuid\"";
        let result: Result<VerificationId, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_invalid_json() {
        let json = "not-json";
        let result: Result<VerificationId, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_partial_eq() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let id1 = VerificationId::from_str(uuid_str).unwrap();
        let id2 = VerificationId::from_str(uuid_str).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_partial_eq_different() {
        let id1 = VerificationId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let id2 = VerificationId::from_str("550e8400-e29b-41d4-a716-446655440001").unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_clone() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let id = VerificationId::from_str(uuid_str).unwrap();
        let cloned = id.clone();
        assert_eq!(id, cloned);
    }

    #[test]
    fn test_hash_trait() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let id1 = VerificationId::from_str(uuid_str).unwrap();
        let id2 = VerificationId::from_str(uuid_str).unwrap();

        let mut hasher1 = DefaultHasher::new();
        id1.hash(&mut hasher1);
        let hash_value1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        id2.hash(&mut hasher2);
        let hash_value2 = hasher2.finish();

        assert_eq!(hash_value1, hash_value2);
    }

    #[test]
    fn test_as_uuid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let verification_id = VerificationId::from_str(uuid_str).unwrap();
        let uuid = verification_id.as_uuid();
        assert_eq!(uuid.to_string(), uuid_str);
    }

    #[test]
    fn test_random_generates_valid_uuid() {
        let id1 = VerificationId::random();
        let id2 = VerificationId::random();

        // Should generate different UUIDs
        assert_ne!(id1, id2);

        // Should be valid UUID format
        assert!(Uuid::parse_str(&id1.to_string()).is_ok());
        assert!(Uuid::parse_str(&id2.to_string()).is_ok());
    }

    #[test]
    fn test_error_display() {
        let error = VerificationIdError::InvalidFormat("test error".to_string());
        assert_eq!(
            error.to_string(),
            "Invalid verification ID format: test error"
        );
    }

    #[test]
    fn test_debug_trait() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let verification_id = VerificationId::from_str(uuid_str).unwrap();
        let debug_str = format!("{:?}", verification_id);
        assert!(debug_str.contains("VerificationId"));
    }
}
