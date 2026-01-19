use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperPage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperSearchRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<WrapperPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperSearchResponse {
    pub items: Vec<WrapperItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperGetRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperGetResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperContentRequest {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperContentResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperErrorResponse {
    pub error: WrapperErrorBody,
}

pub fn validate_search_response(resp: &WrapperSearchResponse) -> Result<(), String> {
    for (idx, item) in resp.items.iter().enumerate() {
        if item.id.trim().is_empty() {
            return Err(format!("items[{}].id is required", idx));
        }
    }
    Ok(())
}

pub fn validate_get_response(resp: &WrapperGetResponse) -> Result<(), String> {
    if resp.id.trim().is_empty() {
        return Err("id is required".to_string());
    }
    Ok(())
}

pub fn validate_content_response(resp: &WrapperContentResponse) -> Result<(), String> {
    if resp.id.trim().is_empty() {
        return Err("id is required".to_string());
    }
    Ok(())
}

pub fn validate_error_response(resp: &WrapperErrorResponse) -> Result<(), String> {
    if resp.error.code.trim().is_empty() {
        return Err("error.code is required".to_string());
    }
    if resp.error.message.trim().is_empty() {
        return Err("error.message is required".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_search_requires_ids() {
        let resp = WrapperSearchResponse {
            items: vec![WrapperItem {
                id: "".into(),
                title: None,
                item_type: None,
                snippet: None,
                updated_at: None,
                source_url: None,
                metadata: None,
            }],
            next_cursor: None,
        };
        assert!(validate_search_response(&resp).is_err());
    }

    #[test]
    fn validate_get_requires_id() {
        let resp = WrapperGetResponse {
            id: "".into(),
            title: None,
            item_type: None,
            updated_at: None,
            size_bytes: None,
            mime_type: None,
            metadata: None,
        };
        assert!(validate_get_response(&resp).is_err());
    }

    #[test]
    fn validate_error_requires_fields() {
        let resp = WrapperErrorResponse {
            error: WrapperErrorBody {
                code: "".into(),
                message: "".into(),
                retry_after_ms: None,
            },
        };
        assert!(validate_error_response(&resp).is_err());
    }
}
