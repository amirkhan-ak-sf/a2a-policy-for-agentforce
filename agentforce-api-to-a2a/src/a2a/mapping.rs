//! A2A <-> Agentforce conversions.
//!
//! Pure functions; no I/O. Tested without spinning up the PDK runtime.

use uuid::Uuid;

use crate::a2a::types::{Artifact, Message, Part, Task, TaskState, TaskStatus};
use crate::agentforce::client::{AgentforceMessage, SendMessageResponse};

/// Build an A2A user-role `Message` from the inbound user text. Used so
/// `Task.history` always contains both turns of the round-trip.
pub fn user_message(text: &str, task_id: &str, message_id: Option<String>) -> Message {
    let mut m = Message::new(
        "user",
        message_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        vec![Part::text(text.to_string())],
    );
    m.task_id = Some(task_id.into());
    m.context_id = Some(task_id.into());
    m
}

/// Build the agent-role `Message` and matching `Artifact` from the
/// Agentforce sync-send response.
///
/// The Agentforce response shape is `{ messages: [{ id, type, message, ... }, ...] }`.
/// We concatenate the `message` text of every "Inform"-style entry; any
/// extra entries (e.g. `Inquire`, `EscalateTo`) are flattened into the
/// same artifact in source order.
pub fn agent_response_to_a2a(
    response: &SendMessageResponse,
    task_id: &str,
) -> (Option<Message>, Option<Artifact>) {
    let texts: Vec<String> = response
        .messages
        .iter()
        .filter_map(|m| extract_message_text(m))
        .collect();

    if texts.is_empty() {
        return (None, None);
    }

    let parts: Vec<Part> = texts.iter().cloned().map(Part::text).collect();

    let agent_message_id = response
        .messages
        .first()
        .and_then(|m| m.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut msg = Message::new("agent", agent_message_id.clone(), parts.clone());
    msg.task_id = Some(task_id.into());
    msg.context_id = Some(task_id.into());

    let artifact = Artifact {
        artifact_id: format!("agent-response-{agent_message_id}"),
        name: Some("agent-response".into()),
        description: None,
        parts,
        metadata: None,
    };

    (Some(msg), Some(artifact))
}

/// Extract the displayable text from one Agentforce message. Falls back to
/// known nested fields when `message` is absent (Agentforce shapes vary by
/// message `type`).
fn extract_message_text(m: &AgentforceMessage) -> Option<String> {
    if let Some(t) = m.message.as_deref() {
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    // Common nested shapes: `result`, `text`, `value`.
    for field in ["result", "text", "value"] {
        if let Some(v) = m.extra.get(field) {
            match v {
                serde_json::Value::String(s) if !s.is_empty() => return Some(s.clone()),
                serde_json::Value::Object(o) => {
                    if let Some(serde_json::Value::String(s)) = o.get("text") {
                        if !s.is_empty() {
                            return Some(s.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Build the final A2A `Task` from the round-trip pieces. State is
/// `completed` for sync sends.
pub fn build_task(
    task_id: String,
    user_msg: Message,
    agent_msg: Option<Message>,
    artifact: Option<Artifact>,
    timestamp: String,
) -> Task {
    let mut task = Task::new(
        task_id,
        TaskStatus {
            state: TaskState::Completed,
            timestamp: Some(timestamp),
            message: None,
        },
    );
    task.history.push(user_msg);
    if let Some(am) = agent_msg {
        task.history.push(am);
    }
    if let Some(art) = artifact {
        task.artifacts.push(art);
    }
    task
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentforce::client::AgentforceMessage;

    fn make_msg(id: &str, text: Option<&str>) -> AgentforceMessage {
        AgentforceMessage {
            id: Some(id.into()),
            kind: Some("Inform".into()),
            message: text.map(|s| s.to_string()),
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn user_message_carries_task_and_context() {
        let m = user_message("hi", "task-1", Some("u1".into()));
        assert_eq!(m.role, "user");
        assert_eq!(m.message_id, "u1");
        assert_eq!(m.task_id.as_deref(), Some("task-1"));
        assert_eq!(m.context_id.as_deref(), Some("task-1"));
    }

    #[test]
    fn user_message_generates_id_when_omitted() {
        let m = user_message("hi", "task-1", None);
        assert!(!m.message_id.is_empty());
    }

    #[test]
    fn agent_response_concatenates_inform_messages() {
        let response = SendMessageResponse {
            messages: vec![
                make_msg("m1", Some("Hello")),
                make_msg("m2", Some("How can I help?")),
            ],
        };
        let (msg, artifact) = agent_response_to_a2a(&response, "task-1");
        let msg = msg.expect("expected agent message");
        let artifact = artifact.expect("expected artifact");
        assert_eq!(msg.parts.len(), 2);
        assert_eq!(artifact.parts.len(), 2);
        assert!(msg.message_id.starts_with("m1") || msg.message_id == "m1");
    }

    #[test]
    fn agent_response_returns_none_when_no_text() {
        let response = SendMessageResponse {
            messages: vec![AgentforceMessage {
                id: Some("m1".into()),
                kind: Some("Acknowledge".into()),
                message: None,
                extra: serde_json::Map::new(),
            }],
        };
        let (msg, artifact) = agent_response_to_a2a(&response, "task-1");
        assert!(msg.is_none());
        assert!(artifact.is_none());
    }

    #[test]
    fn extract_falls_back_to_result_field() {
        let mut extra = serde_json::Map::new();
        extra.insert("result".into(), serde_json::json!("a result"));
        let m = AgentforceMessage {
            id: Some("x".into()),
            kind: Some("Inform".into()),
            message: None,
            extra,
        };
        assert_eq!(extract_message_text(&m).as_deref(), Some("a result"));
    }

    #[test]
    fn extract_falls_back_to_nested_text() {
        let mut extra = serde_json::Map::new();
        extra.insert("value".into(), serde_json::json!({"text": "deep text"}));
        let m = AgentforceMessage {
            id: Some("x".into()),
            kind: Some("Inform".into()),
            message: None,
            extra,
        };
        assert_eq!(extract_message_text(&m).as_deref(), Some("deep text"));
    }

    #[test]
    fn build_task_records_completed_state_and_history() {
        let user = user_message("hello", "t1", None);
        let agent = Message::new("agent", "a1".into(), vec![Part::text("hi back")]);
        let artifact = Artifact {
            artifact_id: "a1".into(),
            name: Some("agent-response".into()),
            description: None,
            parts: vec![Part::text("hi back")],
            metadata: None,
        };
        let task = build_task(
            "t1".into(),
            user,
            Some(agent),
            Some(artifact),
            "2026-05-08T00:00:00Z".into(),
        );
        assert_eq!(task.id, "t1");
        assert_eq!(task.context_id, "t1");
        assert_eq!(task.status.state, TaskState::Completed);
        assert_eq!(task.history.len(), 2);
        assert_eq!(task.artifacts.len(), 1);
    }
}
