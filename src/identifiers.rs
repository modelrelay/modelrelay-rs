//! Strongly-typed identifier newtypes for domain concepts.
//!
//! This module provides foundational identifier types that are used across
//! multiple modules. By centralizing them here, we avoid circular dependencies
//! between domain modules (e.g., `types.rs` depending on `tiers.rs`).
//!
//! ## Macros
//!
//! Three macros are provided for generating identifier types:
//! - `string_id_type!` - For string-wrapped identifiers (ProviderId, TierCode, NodeId)
//! - `uuid_id_type!` - For UUID-wrapped identifiers (RunId, RequestId)
//! - `hex_hash_32!` - For 32-byte hex-encoded hashes (PlanHash, Sha256Hash)
//!
//! ## Usage
//!
//! All types implement `From<&str>`, `From<String>`, and `Into<String>` for
//! easy conversion. They also serialize/deserialize as plain strings.
//!
//! ```ignore
//! use modelrelay::{ProviderId, TierCode};
//!
//! let provider: ProviderId = "anthropic".into();
//! let tier: TierCode = "pro".into();
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

// ============================================================================
// String ID Macro
// ============================================================================

/// Macro to generate string wrapper newtypes with consistent implementations.
///
/// Each generated type:
/// - Trims whitespace from input values
/// - Implements `From<&str>`, `From<String>`, `Into<String>`
/// - Implements `Display` for string formatting
/// - Serializes/deserializes as a plain string
///
/// Usage:
/// ```ignore
/// string_id_type!(ProviderId, "Provider identifier (e.g., \"anthropic\").");
/// string_id_type!(NodeId);  // Auto-generates doc string
/// ```
macro_rules! string_id_type {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Create a new identifier from any string-like value.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into().trim().to_string())
            }

            /// Get the identifier as a string slice.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Check if the identifier is empty (after trimming).
            pub fn is_empty(&self) -> bool {
                self.0.trim().is_empty()
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                $name::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                $name::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.as_str())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self(String::new())
            }
        }
    };
    // Convenience variant without doc string
    ($name:ident) => {
        string_id_type!(
            $name,
            concat!("String identifier for `", stringify!($name), "`.")
        );
    };
}

// Export macro for use in other modules
pub(crate) use string_id_type;

// ============================================================================
// UUID ID Macro
// ============================================================================

/// Macro to generate UUID wrapper newtypes with consistent implementations.
///
/// Each generated type:
/// - Wraps a UUID v4
/// - Provides `new()` for random generation and `from_uuid()` for wrapping existing
/// - Provides `parse()` for parsing from string with validation
/// - Serializes/deserializes transparently as UUID string
/// - Default generates a new random UUID (not nil)
macro_rules! uuid_id_type {
    ($name:ident, $field_name:expr) => {
        #[doc = concat!("UUID identifier for `", stringify!($name), "`.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub uuid::Uuid);

        impl $name {
            /// Creates a new random ID.
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4())
            }

            /// Creates an ID from an existing UUID.
            ///
            /// Useful for deterministic testing where you need predictable IDs.
            pub fn from_uuid(id: uuid::Uuid) -> Self {
                Self(id)
            }

            /// Parses an ID from a string representation.
            pub fn parse(value: &str) -> $crate::errors::Result<Self> {
                let raw = value.trim();
                if raw.is_empty() {
                    return Err($crate::errors::Error::Validation(
                        $crate::errors::ValidationError::new(concat!($field_name, " is required")),
                    ));
                }
                let id = uuid::Uuid::parse_str(raw).map_err(|err| {
                    $crate::errors::Error::Validation(
                        format!(concat!("invalid ", $field_name, ": {}"), err).into(),
                    )
                })?;
                Ok(Self(id))
            }
        }

        impl Default for $name {
            /// Creates a new random ID (not nil UUID).
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

pub(crate) use uuid_id_type;

// ============================================================================
// Hex Hash Macro
// ============================================================================

/// Macro to generate 32-byte hex-encoded hash newtypes.
///
/// Each generated type:
/// - Stores 32 bytes internally
/// - Parses from 64-character hex string
/// - Provides `to_hex()` for string representation
/// - Serializes/deserializes as hex string
macro_rules! hex_hash_32 {
    ($name:ident, $err_name:expr) => {
        #[doc = concat!("32-byte hex hash for `", stringify!($name), "`.")]
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Parses a hash from a 64-character hex string.
            pub fn parse(value: &str) -> $crate::errors::Result<Self> {
                let raw = value.trim();
                if raw.len() != 64 {
                    return Err($crate::errors::Error::Validation(
                        $crate::errors::ValidationError::new(concat!("invalid ", $err_name)),
                    ));
                }
                let bytes = hex::decode(raw).map_err(|err| {
                    $crate::errors::Error::Validation(
                        format!(concat!("invalid ", $err_name, ": {}"), err).into(),
                    )
                })?;
                if bytes.len() != 32 {
                    return Err($crate::errors::Error::Validation(
                        $crate::errors::ValidationError::new(concat!("invalid ", $err_name)),
                    ));
                }
                let mut out = [0u8; 32];
                out.copy_from_slice(&bytes);
                Ok(Self(out))
            }

            /// Returns the hash as a 64-character hex string.
            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.to_hex())
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.to_hex())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                let bytes = hex::decode(s.trim()).map_err(serde::de::Error::custom)?;
                if bytes.len() != 32 {
                    return Err(serde::de::Error::custom(concat!("invalid ", $err_name)));
                }
                let mut out = [0u8; 32];
                out.copy_from_slice(&bytes);
                Ok(Self(out))
            }
        }
    };
}

pub(crate) use hex_hash_32;

// ============================================================================
// Provider Identifier
// ============================================================================

string_id_type!(
    ProviderId,
    "Provider identifier (e.g., \"anthropic\", \"openai\", \"xai\")."
);

// ============================================================================
// Tier Code
// ============================================================================

string_id_type!(
    TierCode,
    "Tier code identifier (e.g., \"free\", \"pro\", \"enterprise\")."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_trims_whitespace() {
        let id: ProviderId = "  anthropic  ".into();
        assert_eq!(id.as_str(), "anthropic");
    }

    #[test]
    fn tier_code_converts_from_string() {
        let code = TierCode::new("pro");
        assert_eq!(code.as_str(), "pro");
        assert!(!code.is_empty());
    }

    #[test]
    fn tier_code_empty_check() {
        let empty = TierCode::new("  ");
        assert!(empty.is_empty());
    }

    #[test]
    fn provider_id_serializes_as_string() {
        let id = ProviderId::new("openai");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"openai\"");
    }

    #[test]
    fn tier_code_deserializes_from_string() {
        let code: TierCode = serde_json::from_str("\"enterprise\"").unwrap();
        assert_eq!(code.as_str(), "enterprise");
    }
}
