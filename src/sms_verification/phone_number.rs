use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::str::FromStr;
use std::sync::OnceLock;

/// A validated phone number in E.164 format.
/// E.164 format: starts with +, followed by 1-15 digits (e.g., +30123456789)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneNumber(String);

impl PhoneNumber {
    pub fn new(phone: &str) -> anyhow::Result<Self> {
        // E.164 format: starts with +, followed by 1-15 digits
        static E164_REGEX: OnceLock<Regex> = OnceLock::new();
        let e164_regex = E164_REGEX.get_or_init(|| {
            Regex::new(r"^\+[1-9]\d{1,14}$")
                .expect("E.164 regex pattern is valid and should compile")
        });

        if e164_regex.is_match(phone) {
            Ok(Self(phone.to_string()))
        } else {
            anyhow::bail!(
                "Invalid phone number format: {}. Phone number must be in E.164 format (e.g., +30123456789)",
                phone
            )
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Display for PhoneNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PhoneNumber {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl Serialize for PhoneNumber {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PhoneNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_phone_numbers() {
        assert!(PhoneNumber::new("+30123456789").is_ok());
        assert!(PhoneNumber::new("+1234567890123").is_ok());
        assert!(PhoneNumber::new("+12").is_ok());
    }

    #[test]
    fn test_invalid_phone_numbers() {
        // Missing +
        assert!(PhoneNumber::new("30123456789").is_err());
        // Starts with +0
        assert!(PhoneNumber::new("+0123456789").is_err());
        // Contains spaces
        assert!(PhoneNumber::new("+30 123 456 789").is_err());
        // Contains hyphens
        assert!(PhoneNumber::new("+30-123-456-789").is_err());
        // Too short (only country code)
        assert!(PhoneNumber::new("+1").is_err());
        // Too long (more than 15 digits)
        assert!(PhoneNumber::new("+1234567890123456").is_err());
    }
}
