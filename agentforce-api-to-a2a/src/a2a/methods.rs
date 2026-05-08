//! JSON-RPC method dispatch for the A2A 0.3.0 surface.
//!
//! Three methods are implemented:
//!
//!   * `message/send` (spec §7.1)
//!   * `tasks/get`    (spec §7.3)
//!   * `tasks/cancel` (spec §7.4)
//!
//! Anything else returns `-32601 Method not found`.

use std::rc::Rc;

use pdk::hl::HttpClient;
use pdk::logger;

use crate::a2a::mapping;
use crate::a2a::types::{
    self, MessageSendParams, Task, TaskIdParams, TaskQueryParams, TaskState, TaskStatus,
};
use crate::agentforce::client::{AgentforceClient, ClientError};
use crate::jsonrpc::{
    JsonRpcError, JsonRpcRequest, JsonRpcSuccess, CONTENT_TYPE_NOT_SUPPORTED, INTERNAL_ERROR,
    INVALID_PARAMS, METHOD_NOT_FOUND, TASK_NOT_CANCELABLE, TASK_NOT_FOUND,
};
use crate::store::task_store::{TaskMeta, TaskStore};

pub struct Dispatcher {
    pub client: Rc<AgentforceClient>,
    pub store: Rc<TaskStore>,
}

impl Dispatcher {
    pub async fn dispatch(
        &self,
        http: &HttpClient,
        request: JsonRpcRequest,
        now_unix: u64,
        timestamp_iso: String,
    ) -> Result<JsonRpcSuccess, JsonRpcError> {
        logger::error!("a2a-trace: dispatch method='{}'", request.method);
        let r = match request.method.as_str() {
            "message/send" => self.handle_message_send(http, request, now_unix, timestamp_iso).await,
            "tasks/get" => self.handle_tasks_get(http, request, now_unix).await,
            "tasks/cancel" => self.handle_tasks_cancel(http, request, now_unix, timestamp_iso).await,
            other => Err(JsonRpcError::new(
                request.id,
                METHOD_NOT_FOUND,
                format!("Method not found: {other}"),
            )),
        };
        logger::error!("a2a-trace: dispatch done, ok={}", r.is_ok());
        r
    }

    async fn handle_message_send(
        &self,
        http: &HttpClient,
        request: JsonRpcRequest,
        now_unix: u64,
        timestamp_iso: String,
    ) -> Result<JsonRpcSuccess, JsonRpcError> {
        let id = request.id.clone();
        let params: MessageSendParams = require_params(&request)?;

        // Reject non-text inputs (file/data parts) until v2 wires them up.
        let text = types::concat_text(&params.message.parts);
        if text.trim().is_empty() {
            return Err(JsonRpcError::new(
                id,
                CONTENT_TYPE_NOT_SUPPORTED,
                "Incompatible content types: only text parts are supported in v1",
            ));
        }

        // Resolve the task / session id. If the client provided one, reuse
        // it; otherwise mint a new session via Agentforce.
        let task_id = match params.message.task_id.clone() {
            Some(id) if !id.is_empty() => id,
            _ => {
                // Avoid runtime randomness in the WASM module. Some Flex
                // Gateway hosts expose limited WASI support, so use the A2A
                // message id as stable caller-provided correlation material.
                let external_session_key = params.message.message_id.clone();
                let session = self
                    .client
                    .start_session(http, &external_session_key, now_unix)
                    .await
                    .map_err(client_error_to_rpc(id.clone()))?;
                session.session_id
            }
        };

        // Sequence counter (per-task, persistent).
        let mut meta = self
            .store
            .get_meta(http, &task_id, now_unix)
            .await
            .unwrap_or_else(|| TaskMeta::initial(now_unix));
        let seq = meta.next_sequence(now_unix);
        self.store.put_meta(http, &task_id, &meta, now_unix).await;

        // Sync send.
        let response = self
            .client
            .send_message(http, &task_id, &text, seq, now_unix)
            .await
            .map_err(client_error_to_rpc(id.clone()))?;

        // Build the response Task.
        let user_msg = mapping::user_message(
            &text,
            &task_id,
            Some(params.message.message_id.clone()),
        );
        let (agent_msg, artifact) = mapping::agent_response_to_a2a(&response, &task_id);
        let task = mapping::build_task(
            task_id.clone(),
            user_msg,
            agent_msg,
            artifact,
            timestamp_iso,
        );

        // Persist the Task.
        if let Ok(bytes) = serde_json::to_vec(&task) {
            self.store.put_task(http, &task_id, &bytes, now_unix).await;
        }

        let value = serde_json::to_value(&task).map_err(|e| {
            JsonRpcError::new(
                id.clone(),
                INTERNAL_ERROR,
                format!("Failed to serialize Task: {e}"),
            )
        })?;
        Ok(JsonRpcSuccess::new(id, value))
    }

    async fn handle_tasks_get(
        &self,
        http: &HttpClient,
        request: JsonRpcRequest,
        now_unix: u64,
    ) -> Result<JsonRpcSuccess, JsonRpcError> {
        let id = request.id.clone();
        let params: TaskQueryParams = require_params(&request)?;
        logger::error!("a2a-trace: tasks/get id='{}'", params.id);
        logger::error!("a2a-trace: tasks/get -> store.get_task");
        let bytes = match self.store.get_task(http, &params.id, now_unix).await {
            Some(b) => {
                logger::error!("a2a-trace: tasks/get store hit, len={}", b.len());
                b
            }
            None => {
                logger::error!("a2a-trace: tasks/get store miss");
                return Err(JsonRpcError::new(
                    id,
                    TASK_NOT_FOUND,
                    "Task not found",
                )
                .with_data(serde_json::json!({ "taskId": params.id })))
            }
        };
        let mut task: Task = serde_json::from_slice(&bytes).map_err(|e| {
            JsonRpcError::new(
                id.clone(),
                INTERNAL_ERROR,
                format!("Persisted Task could not be parsed: {e}"),
            )
        })?;

        // historyLength=0 means "no history". `None` means "all history",
        // which is what we already store. Truncation is from the start
        // (oldest first) so the most recent turns are kept.
        if let Some(n) = params.history_length {
            let n = n as usize;
            if task.history.len() > n {
                let drop = task.history.len() - n;
                task.history = task.history.split_off(drop);
            }
        }

        let value = serde_json::to_value(&task).map_err(|e| {
            JsonRpcError::new(
                id.clone(),
                INTERNAL_ERROR,
                format!("Failed to serialize Task: {e}"),
            )
        })?;
        Ok(JsonRpcSuccess::new(id, value))
    }

    async fn handle_tasks_cancel(
        &self,
        http: &HttpClient,
        request: JsonRpcRequest,
        now_unix: u64,
        timestamp_iso: String,
    ) -> Result<JsonRpcSuccess, JsonRpcError> {
        let id = request.id.clone();
        let params: TaskIdParams = require_params(&request)?;

        let bytes = self
            .store
            .get_task(http, &params.id, now_unix)
            .await
            .ok_or_else(|| {
                JsonRpcError::new(id.clone(), TASK_NOT_FOUND, "Task not found")
                    .with_data(serde_json::json!({ "taskId": params.id }))
            })?;
        let mut task: Task = serde_json::from_slice(&bytes).map_err(|e| {
            JsonRpcError::new(
                id.clone(),
                INTERNAL_ERROR,
                format!("Persisted Task could not be parsed: {e}"),
            )
        })?;

        if task.status.state.is_terminal() {
            return Err(JsonRpcError::new(
                id,
                TASK_NOT_CANCELABLE,
                "Task is in a terminal state and cannot be canceled",
            )
            .with_data(serde_json::json!({
                "taskId": params.id,
                "state": task.status.state,
            })));
        }

        // End the session in Agentforce. Treat upstream 4xx as non-fatal.
        if let Err(e) = self.client.end_session(http, &params.id, now_unix).await {
            logger::warn!("a2a: endSession failed for {}: {e}", params.id);
        }

        task.status = TaskStatus {
            state: TaskState::Canceled,
            timestamp: Some(timestamp_iso),
            message: None,
        };
        if let Ok(b) = serde_json::to_vec(&task) {
            self.store.put_task(http, &params.id, &b, now_unix).await;
        }

        let value = serde_json::to_value(&task).map_err(|e| {
            JsonRpcError::new(
                id.clone(),
                INTERNAL_ERROR,
                format!("Failed to serialize Task: {e}"),
            )
        })?;
        Ok(JsonRpcSuccess::new(id, value))
    }
}

fn require_params<T>(request: &JsonRpcRequest) -> Result<T, JsonRpcError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let params = request
        .params
        .clone()
        .ok_or_else(|| JsonRpcError::new(request.id.clone(), INVALID_PARAMS, "params is required"))?;
    serde_json::from_value(params).map_err(|e| {
        JsonRpcError::new(
            request.id.clone(),
            INVALID_PARAMS,
            format!("Invalid params: {e}"),
        )
    })
}

fn client_error_to_rpc(id: Option<serde_json::Value>) -> impl FnOnce(ClientError) -> JsonRpcError {
    move |err| {
        let (code, reason, data) = match &err {
            ClientError::AuthRejected => (
                INTERNAL_ERROR,
                "Agentforce authentication rejected".to_string(),
                Some(serde_json::json!({ "reason": "agentforce_auth_rejected" })),
            ),
            ClientError::HttpStatus { status, operation, .. } => (
                INTERNAL_ERROR,
                format!("Agentforce {operation} returned HTTP {status}"),
                Some(serde_json::json!({
                    "reason": "agentforce_http_error",
                    "status": status,
                    "operation": operation,
                })),
            ),
            ClientError::Auth(_) => (
                INTERNAL_ERROR,
                "Agentforce token endpoint failure".to_string(),
                Some(serde_json::json!({ "reason": "agentforce_token_failure" })),
            ),
            ClientError::Transport { .. } => (
                INTERNAL_ERROR,
                "Agentforce transport error".to_string(),
                Some(serde_json::json!({ "reason": "agentforce_transport_error" })),
            ),
            ClientError::BadJson(_) => (
                INTERNAL_ERROR,
                "Agentforce response was not valid JSON".to_string(),
                Some(serde_json::json!({ "reason": "agentforce_bad_json" })),
            ),
            ClientError::MissingField(field) => (
                INTERNAL_ERROR,
                format!("Agentforce response missing field: {field}"),
                Some(serde_json::json!({
                    "reason": "agentforce_missing_field",
                    "field": field,
                })),
            ),
        };
        let mut e = JsonRpcError::new(id, code, reason);
        if let Some(d) = data {
            e = e.with_data(d);
        }
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonrpc::JsonRpcRequest;

    #[test]
    fn require_params_rejects_missing() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tasks/get".into(),
            params: None,
        };
        let err: JsonRpcError = require_params::<TaskIdParams>(&req).unwrap_err();
        assert_eq!(err.error.code, INVALID_PARAMS);
    }

    #[test]
    fn require_params_rejects_garbage() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tasks/get".into(),
            params: Some(serde_json::json!({ "wrongField": 1 })),
        };
        let err: JsonRpcError = require_params::<TaskIdParams>(&req).unwrap_err();
        assert_eq!(err.error.code, INVALID_PARAMS);
    }

    #[test]
    fn require_params_parses_valid_input() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "tasks/get".into(),
            params: Some(serde_json::json!({ "id": "task-1" })),
        };
        let p: TaskIdParams = require_params(&req).unwrap();
        assert_eq!(p.id, "task-1");
    }

    #[test]
    fn client_error_to_rpc_carries_reason_for_auth_rejected() {
        let mapper = client_error_to_rpc(Some(serde_json::json!(1)));
        let err = mapper(ClientError::AuthRejected);
        assert_eq!(err.error.code, INTERNAL_ERROR);
        assert_eq!(
            err.error.data.unwrap()["reason"],
            "agentforce_auth_rejected"
        );
    }

    #[test]
    fn client_error_to_rpc_carries_status_on_http_error() {
        let mapper = client_error_to_rpc(Some(serde_json::json!(1)));
        let err = mapper(ClientError::HttpStatus {
            operation: "sendMessage",
            status: 503,
            body: "service unavailable".into(),
        });
        let data = err.error.data.unwrap();
        assert_eq!(data["status"], 503);
        assert_eq!(data["operation"], "sendMessage");
    }
}
