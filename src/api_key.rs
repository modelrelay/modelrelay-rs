use serde::{Deserialize, Serialize};

use crate::errors::{Error, Result, ValidationError};

const SECRET_PREFIX: &str = "mr_sk_";

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretKey(String);

impl SecretKey {
    pub fn parse(raw: impl AsRef<str>) -> Result<Self> {
        let value = raw.as_ref().trim();
        if value.starts_with(SECRET_PREFIX) && value.len() > SECRET_PREFIX.len() {
            return Ok(Self(value.to_string()));
        }
        Err(Error::Validation(ValidationError::new(
            "invalid secret key (expected mr_sk_*)",
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ApiKey {
    Secret(SecretKey),
}

impl ApiKey {
    pub fn parse(raw: impl AsRef<str>) -> Result<Self> {
        let value = raw.as_ref().trim();
        if value.starts_with(SECRET_PREFIX) {
            return Ok(ApiKey::Secret(SecretKey::parse(value)?));
        }
        Err(Error::Validation(ValidationError::new(
            "invalid API key (expected mr_sk_*)",
        )))
    }

    pub fn as_str(&self) -> &str {
        match self {
            ApiKey::Secret(k) => k.as_str(),
        }
    }

    pub fn is_secret(&self) -> bool {
        matches!(self, ApiKey::Secret(_))
    }

    pub fn as_secret(&self) -> Option<&SecretKey> {
        match self {
            ApiKey::Secret(k) => Some(k),
        }
    }
}

impl std::fmt::Display for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<SecretKey> for ApiKey {
    fn from(value: SecretKey) -> Self {
        ApiKey::Secret(value)
    }
}
