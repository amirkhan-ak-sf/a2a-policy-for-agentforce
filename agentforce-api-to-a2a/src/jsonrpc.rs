//! JSON-RPC 2.0 envelope plus the A2A-specific error codes used in the
//! response when one of our handlers needs to surface a domain-level error.
//!
//! See A2A 0.3.0 Specification §6.11 (JSON-RPC Structures) and §8 (Error
//! Handling).

use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 protocol version literal.
pub const JSONRPC_VERSION: &str = "2.0";

/// A2A-specific error codes (spec §8.2).
pub const TASK_NOT_FOUND: i32 = -32001;
pub const TASK_NOT_CANCELABLE: i32 = -32002;
#[allow(dead_code)]
pub const PUSH_NOTIFICATION_NOT_SUPPORTED: i32 = -32003;
#[allow(dead_code)]
pub const UNSUPPORTED_OPERATION: i32 = -32004;
pub const CONTENT_TYPE_NOT_SUPPORTED: i32 = -32005;
#[allow(dead_code)]
pub const INVALID_AGENT_RESPONSE: i32 = -32006;

// Standard JSON-RPC error codes (spec §8.1).
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

/// Incoming JSON-RPC request envelope. We capture `id` as a raw `Value`
/// because A2A allows strings, numbers, or null and we must echo the same
/// type back unchanged.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// Outgoing success response. Uses an explicit shape rather than
/// `#[serde(untagged)]` so success and error responses can never collide.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcSuccess {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub error: JsonRpcErrorObject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcSuccess {
    pub fn new(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id: id.unwrap_or(serde_json::Value::Null),
            result,
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        serde_json::to_vec(&self).expect("serializing JsonRpcSuccess never fails")
    }
}

impl JsonRpcError {
    pub fn new(id: Option<serde_json::Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id: id.unwrap_or(serde_json::Value::Null),
            error: JsonRpcErrorObject {
                code,
                message: message.into(),
                data: None,
            },
        }
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.error.data = Some(data);
        self
    }

    pub fn into_bytes(self) -> Vec<u8> {
        serde_json::to_vec(&self).expect("serializing JsonRpcError never fails")
    }
}

/// Convenience: parse a JSON-RPC request body. Returns the JSON-RPC error
/// to send back if parsing fails.
pub fn parse_request(body: &[u8]) -> Result<JsonRpcRequest, JsonRpcError> {
    let raw: serde_json::Value = serde_json::from_slice(body).map_err(|e| {
        JsonRpcError::new(None, PARSE_ERROR, "Parse error").with_data(serde_json::json!({
            "reason": e.to_string()
        }))
    })?;

    let id = raw.get("id").cloned();
    let request: JsonRpcRequest = serde_json::from_value(raw).map_err(|e| {
        JsonRpcError::new(id.clone(), INVALID_REQUEST, "Invalid Request").with_data(
            serde_json::json!({
                "reason": e.to_string()
            }),
        )
    })?;

    if request.jsonrpc != JSONRPC_VERSION {
        return Err(JsonRpcError::new(
            request.id,
            INVALID_REQUEST,
            "Invalid Request: jsonrpc must be \"2.0\"",
        ));
    }
    if request.method.is_empty() {
        return Err(JsonRpcError::new(
            request.id,
            INVALID_REQUEST,
            "Invalid Request: method is required",
        ));
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_request() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"message/send"}"#;
        let req = parse_request(body).unwrap();
        assert_eq!(req.method, "message/send");
        assert_eq!(req.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn rejects_garbage() {
        let body = b"<html>nope</html>";
        let err = parse_request(body).unwrap_err();
        assert_eq!(err.error.code, PARSE_ERROR);
        // The id should be Null because we couldn't even parse the body.
        assert!(err.id.is_null());
    }

    #[test]
    fn rejects_wrong_jsonrpc_version() {
        let body = br#"{"jsonrpc":"1.0","id":2,"method":"x"}"#;
        let err = parse_request(body).unwrap_err();
        assert_eq!(err.error.code, INVALID_REQUEST);
        assert_eq!(err.id, serde_json::json!(2));
    }

    #[test]
    fn rejects_missing_method() {
        let body = br#"{"jsonrpc":"2.0","id":3}"#;
        let err = parse_request(body).unwrap_err();
        assert_eq!(err.error.code, INVALID_REQUEST);
    }

    #[test]
    fn echoes_string_id() {
        let body = br#"{"jsonrpc":"2.0","id":"abc","method":"x"}"#;
        let req = parse_request(body).unwrap();
        assert_eq!(req.id, Some(serde_json::json!("abc")));
    }

    #[test]
    fn success_serializes_round_trip() {
        let s = JsonRpcSuccess::new(Some(serde_json::json!(1)), serde_json::json!({"ok":true}));
        let bytes = s.into_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"]["ok"], true);
        assert!(v.get("error").is_none());
    }

    #[test]
    fn error_serializes_with_optional_data() {
        let e = JsonRpcError::new(Some(serde_json::json!("k")), TASK_NOT_FOUND, "Task not found")
            .with_data(serde_json::json!({"taskId":"x"}));
        let bytes = e.into_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], "k");
        assert_eq!(v["error"]["code"], TASK_NOT_FOUND);
        assert_eq!(v["error"]["data"]["taskId"], "x");
        assert!(v.get("result").is_none());
    }

    #[test]
    fn error_omits_data_when_none() {
        let e = JsonRpcError::new(None, METHOD_NOT_FOUND, "Method not found");
        let bytes = e.into_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["error"].get("data").is_none());
    }
}
