use std::{fmt::Display, str::FromStr, sync::OnceLock};

use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::prelude_api::PreludeBlockedReason;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Raw request from JSON
#[derive(Debug, Deserialize)]
pub struct CreateVerificationRequestRaw {
    pub phone_number: String,
}

/// Validated request
#[derive(Debug)]
pub struct CreateVerificationRequest {
    pub phone_number: PhoneNumber,
}

impl TryFrom<CreateVerificationRequestRaw> for CreateVerificationRequest {
    type Error = SmsVerificationError;

    fn try_from(raw: CreateVerificationRequestRaw) -> Result<Self, Self::Error> {
        let phone_number = PhoneNumber::new(&raw.phone_number)
            .map_err(|_| SmsVerificationError::InvalidPhoneNumber(raw.phone_number))?;
        Ok(Self { phone_number })
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CreateVerificationResponse {
    Success,
    Retry,
    Blocked { reason: PreludeBlockedReason },
}

/// Raw request from JSON
#[derive(Debug, Deserialize)]
pub struct SendCodeRequestRaw {
    pub phone_number: String,
    pub code: String,
}

/// Validated request
#[derive(Debug)]
pub struct SendCodeRequest {
    pub phone_number: PhoneNumber,
    pub code: Code,
}

impl TryFrom<SendCodeRequestRaw> for SendCodeRequest {
    type Error = SmsVerificationError;

    fn try_from(raw: SendCodeRequestRaw) -> Result<Self, Self::Error> {
        let phone_number = PhoneNumber::new(&raw.phone_number)
            .map_err(|_| SmsVerificationError::InvalidPhoneNumber(raw.phone_number.clone()))?;
        let code = Code::new(&raw.code).map_err(|_| SmsVerificationError::InvalidCode(raw.code))?;
        Ok(Self { phone_number, code })
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SendCodeResponse {
    Success {
        signup_code: String,
        homeserver_pubky: String,
    },
    Failure,
    ExpiredOrNotFound,
}

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

/// A validated verification code.
/// Must be exactly 6 digits (0-9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code(String);

impl Code {
    pub fn new(code: &str) -> anyhow::Result<Self> {
        // Exactly 6 digits (0-9)
        static CODE_REGEX: OnceLock<Regex> = OnceLock::new();
        let code_regex = CODE_REGEX.get_or_init(|| {
            Regex::new(r"^\d{6}$").expect("Code regex pattern is valid and should compile")
        });

        if code_regex.is_match(code) {
            Ok(Self(code.to_string()))
        } else {
            anyhow::bail!(
                "Invalid code format: {}. Code must be exactly 6 digits (0-9)",
                code
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

impl Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Code {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl Serialize for Code {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Code {
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
    fn test_code() {
        assert!(Code::new("000000").is_ok());
        assert!(Code::new("999999").is_ok());
        assert!(Code::new("012345").is_ok());
        // Too short
        assert!(Code::new("123").is_err());
        // Too long
        assert!(Code::new("1234567").is_err());
        // Contains letters
        assert!(Code::new("12345a").is_err());
        // Contains special characters
        assert!(Code::new("12345-").is_err());
        // Empty string
        assert!(Code::new("").is_err());
        // Contains spaces
        assert!(Code::new("12 456").is_err());
    }

    #[test]
    fn test_phone_number() {
        assert!(PhoneNumber::new("+30123456789").is_ok());
        assert!(PhoneNumber::new("+1234567890123").is_ok());
        assert!(PhoneNumber::new("+12").is_ok());
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
