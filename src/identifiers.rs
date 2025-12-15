//! Strongly-typed identifier newtypes for domain concepts.
//!
//! This module provides foundational identifier types that are used across
//! multiple modules. By centralizing them here, we avoid circular dependencies
//! between domain modules (e.g., `types.rs` depending on `tiers.rs`).
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

/// Macro to generate string wrapper newtypes with consistent implementations.
///
/// Each generated type:
/// - Trims whitespace from input values
/// - Implements `From<&str>`, `From<String>`, `Into<String>`
/// - Implements `Display` for string formatting
/// - Serializes/deserializes as a plain string
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
}

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
