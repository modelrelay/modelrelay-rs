#![cfg(any(feature = "client", feature = "blocking"))]

use reqwest::{StatusCode, header::HeaderMap};

use crate::{
    REQUEST_ID_HEADER,
    errors::{APIError, Error},
};

/// Optional headers and metadata for proxy calls.
#[derive(Clone, Default)]
pub struct ProxyOptions {
    pub request_id: Option<String>,
    pub headers: Vec<(String, String)>,
}

impl ProxyOptions {
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }
}

pub(crate) fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(REQUEST_ID_HEADER) {
        if let Ok(s) = value.to_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(value) = headers.get("X-Request-Id") {
        if let Ok(s) = value.to_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

pub(crate) fn parse_api_error_parts(
    status: StatusCode,
    headers: &HeaderMap,
    body: String,
) -> Error {
    let request_id = request_id_from_headers(headers);
    let status_code = status.as_u16();
    let status_text = status
        .canonical_reason()
        .unwrap_or("request failed")
        .to_string();

    if body.is_empty() {
        return APIError {
            status: status_code,
            code: None,
            message: status_text,
            request_id,
            fields: Vec::new(),
        }
        .into();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(err_obj) = value.get("error").and_then(|v| v.as_object()) {
            let code = err_obj
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let message = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| status_text.clone());
            let fields = err_obj
                .get("fields")
                .and_then(|v| {
                    serde_json::from_value::<Vec<crate::errors::FieldError>>(v.clone()).ok()
                })
                .unwrap_or_default();
            let status_override = err_obj
                .get("status")
                .and_then(|v| v.as_u64())
                .map(|v| v as u16)
                .unwrap_or(status_code);
            return APIError {
                status: status_override,
                code,
                message,
                request_id: value
                    .get("request_id")
                    .or_else(|| value.get("requestId"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or(request_id),
                fields,
            }
            .into();
        }

        if let Some(message) = value.get("message").and_then(|v| v.as_str()) {
            let code = value
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let fields = value
                .get("fields")
                .and_then(|v| {
                    serde_json::from_value::<Vec<crate::errors::FieldError>>(v.clone()).ok()
                })
                .unwrap_or_default();
            return APIError {
                status: status_code,
                code,
                message: message.to_string(),
                request_id,
                fields,
            }
            .into();
        }
    }

    APIError {
        status: status_code,
        code: None,
        message: body,
        request_id,
        fields: Vec::new(),
    }
    .into()
}
