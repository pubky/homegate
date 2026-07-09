#[derive(Debug)]
pub struct GoogleVerificationRequest {
    pub google_id_token: String,
}

impl<'de> serde::Deserialize<'de> for GoogleVerificationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Request {
            google_id_token: String,
        }

        let request = Request::deserialize(deserializer)?;
        if request.google_id_token.trim().is_empty() {
            return Err(serde::de::Error::custom("googleIdToken must not be empty"));
        }

        Ok(Self {
            google_id_token: request.google_id_token,
        })
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleVerificationResponse {
    pub signup_code: String,
    pub homeserver_pubky: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_verification_request_rejects_unknown_fields() {
        let err = serde_json::from_str::<GoogleVerificationRequest>(
            r#"{"googleIdToken":"token","driveAccessToken":"drive-token"}"#,
        )
        .unwrap_err();

        assert!(err.is_data());
    }

    #[test]
    fn google_verification_request_rejects_empty_token() {
        let err = serde_json::from_str::<GoogleVerificationRequest>(r#"{"googleIdToken":"   "}"#)
            .unwrap_err();

        assert!(err.is_data());
    }
}
