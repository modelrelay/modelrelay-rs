//! Validation tests to ensure generated types align with hand-written types.
//!
//! These tests catch drift between the OpenAPI spec and hand-written SDK types.

#[cfg(test)]
mod validation {
    use crate::generated::*;

    /// Test that generated Customer type deserializes correctly.
    #[test]
    fn generated_customer_deserializes() {
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "project_id": "550e8400-e29b-41d4-a716-446655440001",
            "external_id": "user_123",
            "email": "test@example.com",
            "metadata": {"key": "value"},
            "created_at": "2024-01-15T10:30:00Z",
            "updated_at": "2024-01-15T10:30:00Z"
        }"#;

        let customer: Customer =
            serde_json::from_str(json).expect("Failed to deserialize Customer");
        assert_eq!(
            customer.external_id.as_ref().map(|v| v.to_string()),
            Some("user_123".to_string())
        );
        assert_eq!(customer.email, Some("test@example.com".to_string()));
    }

    /// Test that generated Tier type deserializes correctly.
    #[test]
    fn generated_tier_deserializes() {
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "project_id": "550e8400-e29b-41d4-a716-446655440001",
            "tier_code": "pro",
            "display_name": "Pro Plan",
            "spend_limit_cents": 10000,
            "created_at": "2024-01-15T10:30:00Z",
            "updated_at": "2024-01-15T10:30:00Z"
        }"#;

        let tier: Tier = serde_json::from_str(json).expect("Failed to deserialize Tier");
        assert_eq!(
            tier.tier_code.as_ref().map(|v| v.to_string()),
            Some("pro".to_string())
        );
        assert_eq!(
            tier.display_name.as_ref().map(|v| v.to_string()),
            Some("Pro Plan".to_string())
        );
    }

    /// Test that generated Project type deserializes correctly.
    #[test]
    fn generated_project_deserializes() {
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "owner_id": "550e8400-e29b-41d4-a716-446655440001",
            "name": "My Project",
            "description": "A test project",
            "created_at": "2024-01-15T10:30:00Z",
            "updated_at": "2024-01-15T10:30:00Z"
        }"#;

        let project: Project = serde_json::from_str(json).expect("Failed to deserialize Project");
        assert_eq!(project.name, Some("My Project".to_string()));
    }

    /// Test that generated ApiError type deserializes correctly.
    #[test]
    fn generated_api_error_deserializes() {
        let json = r#"{
            "error": "validation_error",
            "code": "INVALID_INPUT",
            "message": "The input was invalid"
        }"#;

        let error: ApiError = serde_json::from_str(json).expect("Failed to deserialize ApiError");
        assert_eq!(error.code, "INVALID_INPUT");
        assert_eq!(error.message, "The input was invalid");
    }

    /// Test that generated types can round-trip through JSON.
    #[test]
    fn generated_types_roundtrip() {
        // Start with JSON, deserialize, then serialize back
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "project_id": "550e8400-e29b-41d4-a716-446655440001",
            "external_id": "user_123",
            "email": "test@example.com"
        }"#;

        let customer: Customer = serde_json::from_str(json).expect("Failed to deserialize");
        let serialized = serde_json::to_string(&customer).expect("Failed to serialize");
        let roundtripped: Customer =
            serde_json::from_str(&serialized).expect("Failed to roundtrip");

        assert_eq!(customer.external_id, roundtripped.external_id);
        assert_eq!(customer.email, roundtripped.email);
    }
}
