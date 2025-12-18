use std::{fmt::Display, str::FromStr};

/// Error type for payment hash parse operations
#[derive(thiserror::Error, Debug)]
pub enum PaymentHashError {
    #[error("Payment hash must be exactly 64 hex characters, got {0} characters")]
    InvalidLength(usize),
    #[error("Payment hash must contain only hex characters (0-9, a-f, A-F)")]
    InvalidCharacters,
}

/// Lightning network payment hash.
/// Consists of 64 hex characters aka 32 bytes, and is case insensitive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaymentHash(String);

impl PaymentHash {
    pub fn new(payment_hash: &str) -> Result<Self, PaymentHashError> {
        if payment_hash.len() != 64 {
            return Err(PaymentHashError::InvalidLength(payment_hash.len()));
        }
        if !payment_hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(PaymentHashError::InvalidCharacters);
        }
        Ok(Self(payment_hash.to_ascii_lowercase()))
    }

    /// Returns a string slice of the payment hash
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Generates a random payment hash
    #[cfg(test)]
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).expect("Randomness generation should not fail");
        let value = hex::encode(bytes);
        Self::new(&value).expect("Should never fail")
    }
}

impl Display for PaymentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PaymentHash {
    type Err = PaymentHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl serde::Serialize for PaymentHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for PaymentHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for PaymentHash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl sqlx::Type<sqlx::Postgres> for PaymentHash {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        // Use bpchar (blank-padded char) which is PostgreSQL's internal name for CHAR
        // This makes it compatible with CHAR(n) columns
        sqlx::postgres::PgTypeInfo::with_name("bpchar")
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for PaymentHash {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        // Decode as String first, which handles both CHAR and TEXT
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        // Trim whitespace that CHAR columns may have (CHAR is blank-padded)
        let s = s.trim_end();
        Self::new(s).map_err(|e| sqlx::Error::Decode(e.to_string().into()).into())
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for PaymentHash {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        let s: &str = self.as_str();
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&s, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_payment_hash() {
        let valid_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let payment_hash = PaymentHash::new(valid_hash).unwrap();
        assert_eq!(payment_hash.as_ref(), valid_hash);
    }

    #[test]
    fn test_valid_payment_hash_uppercase_converted_to_lowercase() {
        let uppercase_hash = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let payment_hash = PaymentHash::new(uppercase_hash).unwrap();
        assert_eq!(
            payment_hash.as_ref(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn test_invalid_length_too_short() {
        let short_hash = "0123456789abcdef";
        let result = PaymentHash::new(short_hash);
        assert!(result.is_err());
        match result.unwrap_err() {
            PaymentHashError::InvalidLength(len) => assert_eq!(len, 16),
            _ => panic!("Expected InvalidLength error"),
        }
    }

    #[test]
    fn test_invalid_length_too_long() {
        let long_hash =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = PaymentHash::new(long_hash);
        assert!(result.is_err());
        match result.unwrap_err() {
            PaymentHashError::InvalidLength(len) => assert_eq!(len, 80),
            _ => panic!("Expected InvalidLength error"),
        }
    }

    #[test]
    fn test_invalid_length_empty() {
        let empty_hash = "";
        let result = PaymentHash::new(empty_hash);
        assert!(result.is_err());
        match result.unwrap_err() {
            PaymentHashError::InvalidLength(len) => assert_eq!(len, 0),
            _ => panic!("Expected InvalidLength error"),
        }
    }

    #[test]
    fn test_invalid_characters_non_hex() {
        let invalid_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg";
        let result = PaymentHash::new(invalid_hash);
        assert!(result.is_err());
        match result.unwrap_err() {
            PaymentHashError::InvalidCharacters => {}
            _ => panic!("Expected InvalidCharacters error"),
        }
    }

    #[test]
    fn test_invalid_characters_with_special_chars() {
        let invalid_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde@";
        let result = PaymentHash::new(invalid_hash);
        assert!(result.is_err());
        match result.unwrap_err() {
            PaymentHashError::InvalidCharacters => {}
            _ => panic!("Expected InvalidCharacters error"),
        }
    }

    #[test]
    fn test_display_trait() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let payment_hash = PaymentHash::new(hash).unwrap();
        assert_eq!(format!("{}", payment_hash), hash);
    }

    #[test]
    fn test_from_str_valid() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let payment_hash: PaymentHash = hash.parse().unwrap();
        assert_eq!(payment_hash.as_ref(), hash);
    }

    #[test]
    fn test_from_str_invalid() {
        let invalid_hash = "invalid";
        let result: Result<PaymentHash, _> = invalid_hash.parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let payment_hash = PaymentHash::new(hash).unwrap();
        let serialized = serde_json::to_string(&payment_hash).unwrap();
        assert_eq!(serialized, format!("\"{}\"", hash));
    }

    #[test]
    fn test_deserialize_valid() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let json = format!("\"{}\"", hash);
        let payment_hash: PaymentHash = serde_json::from_str(&json).unwrap();
        assert_eq!(payment_hash.as_ref(), hash);
    }

    #[test]
    fn test_deserialize_invalid_length() {
        let json = "\"0123456789abcdef\"";
        let result: Result<PaymentHash, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_invalid_characters() {
        let json = "\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg\"";
        let result: Result<PaymentHash, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_as_ref_str() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let payment_hash = PaymentHash::new(hash).unwrap();
        assert_eq!(payment_hash.as_ref(), hash);
    }

    #[test]
    fn test_as_str() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let payment_hash = PaymentHash::new(hash).unwrap();
        assert_eq!(payment_hash.as_str(), hash);
    }

    #[test]
    fn test_partial_eq() {
        let hash1 =
            PaymentHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let hash2 =
            PaymentHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_partial_eq_different() {
        let hash1 =
            PaymentHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let hash2 =
            PaymentHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdee")
                .unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_clone() {
        let hash =
            PaymentHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let cloned = hash.clone();
        assert_eq!(hash, cloned);
    }

    #[test]
    fn test_hash_trait() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let hash1 =
            PaymentHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let hash2 =
            PaymentHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();

        let mut hasher1 = DefaultHasher::new();
        hash1.hash(&mut hasher1);
        let hash_value1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        hash2.hash(&mut hasher2);
        let hash_value2 = hasher2.finish();

        assert_eq!(hash_value1, hash_value2);
    }

    #[test]
    fn test_error_display_invalid_length() {
        let error = PaymentHashError::InvalidLength(32);
        assert_eq!(
            error.to_string(),
            "Payment hash must be exactly 64 hex characters, got 32 characters"
        );
    }

    #[test]
    fn test_error_display_invalid_characters() {
        let error = PaymentHashError::InvalidCharacters;
        assert_eq!(
            error.to_string(),
            "Payment hash must contain only hex characters (0-9, a-f, A-F)"
        );
    }
}
