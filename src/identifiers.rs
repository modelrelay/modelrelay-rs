//! Strongly-typed identifier newtypes for domain concepts.
//!
//! This module re-exports identifier types from the generated OpenAPI spec.
//! Types are generated with built-in validation from the OpenAPI schema.
//!
//! ## Usage
//!
//! ```ignore
//! use modelrelay::{ModelId, ProviderId, TierCode};
//!
//! // ProviderId is an enum - use variants directly
//! let provider = ProviderId::Anthropic;
//!
//! // TierCode validates pattern on parse
//! let tier: TierCode = "pro".parse().expect("valid tier code");
//!
//! // ModelId is a validated string newtype
//! let model: ModelId = "claude-sonnet-4-20250514".parse().expect("valid model id");
//! ```

// Re-export generated types (single source of truth from OpenAPI spec)
// Note: ModelId and ProviderId are also exported via workflow module at crate root.
pub use crate::generated::{ProviderId, TierCode};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_serializes_as_string() {
        let id = ProviderId::Anthropic;
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"anthropic\"");
    }

    #[test]
    fn provider_id_from_string() {
        let id: ProviderId = "openai".parse().unwrap();
        assert_eq!(id, ProviderId::Openai);
    }

    #[test]
    fn tier_code_deserializes_from_string() {
        let code: TierCode = serde_json::from_str("\"enterprise\"").unwrap();
        assert_eq!(code.to_string(), "enterprise");
    }

    #[test]
    fn tier_code_validates_pattern() {
        // Valid: starts with lowercase, alphanumeric + hyphen/underscore
        assert!("pro".parse::<TierCode>().is_ok());
        assert!("free-tier".parse::<TierCode>().is_ok());

        // Invalid: starts with uppercase or number
        assert!("Pro".parse::<TierCode>().is_err());
        assert!("123".parse::<TierCode>().is_err());
    }
}
