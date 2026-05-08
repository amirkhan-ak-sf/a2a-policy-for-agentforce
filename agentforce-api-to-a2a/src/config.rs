//! Typed view over the policy configuration.
//!
//! `pdk build` regenerates `src/generated/config.rs` from `gcl.yaml`. This
//! module wraps the generated struct in a normalized form (defaults applied,
//! enums parsed, agent-card discriminator validated) so the rest of the
//! crate works with a clean strongly-typed `PolicyConfig`.

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("unsupported protocolVersion: {0} (expected 0.3.0)")]
    UnsupportedProtocolVersion(String),

    #[error("unsupported agentCardSource: {0}")]
    UnsupportedAgentCardSource(String),

    #[error("publicBaseUrl is required")]
    MissingPublicBaseUrl,

    #[error("consumerKey is required")]
    MissingConsumerKey,

    #[error("consumerSecret is required")]
    MissingConsumerSecret,

    #[error("agentId is required")]
    MissingAgentId,

    #[error("anypointOrgId is required")]
    MissingAnypointOrgId,

    #[error("anypointEnvId is required")]
    MissingAnypointEnvId,

    #[error("objectStoreId is required")]
    MissingObjectStoreId,

    #[error(
        "agentCardSource is 'inline_json' or 'file' but agentCardJson is empty"
    )]
    AgentCardJsonEmpty,

    #[error("agentCardSource is 'url' but no agentCardUrl Service is registered")]
    AgentCardUrlNotRegistered,

    #[error("agentCardSkillsJson must be a JSON array")]
    AgentCardSkillsNotArray,

    #[error(
        "agentCardSecuritySchemesJson must be a JSON object"
    )]
    AgentCardSecuritySchemesNotObject,

    #[error("agentCardOverrideJson must be a JSON object")]
    AgentCardOverrideNotObject,

    #[error("agentCardJson must be a JSON object")]
    AgentCardJsonNotObject,

    #[error(
        "agentCardProviderOrganization and agentCardProviderUrl must be set together (A2A 0.3.0 §5.5 requires AgentProvider.organization and AgentProvider.url) — leave both blank to omit the provider block"
    )]
    AgentCardProviderHalfSet,

    #[error("invalid JSON in {field}: {error}")]
    InvalidJson { field: &'static str, error: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolVersion {
    V0_3_0,
}

impl ProtocolVersion {
    pub fn parse(s: &str) -> Result<Self, ConfigError> {
        match s {
            "0.3.0" => Ok(Self::V0_3_0),
            other => Err(ConfigError::UnsupportedProtocolVersion(other.to_string())),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V0_3_0 => "0.3.0",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCardSource {
    InlineJson,
    Url,
    Structured,
    File,
}

impl AgentCardSource {
    pub fn parse(s: &str) -> Result<Self, ConfigError> {
        Ok(match s {
            "inline_json" => Self::InlineJson,
            "url" => Self::Url,
            "structured" => Self::Structured,
            "file" => Self::File,
            other => return Err(ConfigError::UnsupportedAgentCardSource(other.to_string())),
        })
    }
}

/// Structured (form-field) snapshot of the AgentCard. Empty/None values are
/// not added to the resulting JSON object so they don't override good
/// defaults coming from a deep-merge.
#[derive(Debug, Clone, Default)]
pub struct StructuredCardConfig {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
    pub provider_organization: Option<String>,
    pub provider_url: Option<String>,
    pub capabilities_streaming: bool,
    pub capabilities_push_notifications: bool,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: serde_json::Value,
    pub security_schemes: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct PolicyConfig {
    // Agentforce
    pub consumer_key: String,
    pub consumer_secret: String,
    pub agent_id: String,
    pub bypass_user: bool,
    pub cache_safety_margin_seconds: u32,

    // A2A
    pub protocol_version: ProtocolVersion,
    pub a2a_rpc_path: String,
    pub public_base_url: String,
    pub strict_mode: bool,

    // OS v2
    pub anypoint_client_id: String,
    pub anypoint_client_secret: String,
    pub anypoint_org_id: String,
    pub anypoint_env_id: String,
    pub object_store_id: String,
    pub auto_create_store: bool,
    pub disable_object_store: bool,
    pub object_store_ttl_seconds: u32,
    pub task_hot_cache_ttl_seconds: u32,
    pub task_store_timeout_ms: u64,

    // Agent card
    pub agent_card_source: AgentCardSource,
    pub agent_card_json: Option<String>,
    pub structured_card: StructuredCardConfig,
    pub agent_card_override: Option<serde_json::Value>,
}

/// Raw, untyped view of the configuration. The generated codegen produces a
/// near-identical struct with extra Service fields; we narrow to this
/// host-testable shape so all defaulting/validation logic lives in one place
/// and is unit-testable without the full PDK runtime.
#[derive(Debug, Clone, Default)]
pub struct RawConfig {
    pub consumer_key: Option<String>,
    pub consumer_secret: Option<String>,
    pub agent_id: Option<String>,
    pub bypass_user: Option<bool>,
    pub cache_safety_margin_seconds: Option<i64>,

    pub protocol_version: Option<String>,
    pub a2a_rpc_path: Option<String>,
    pub public_base_url: Option<String>,
    pub strict_mode: Option<bool>,

    pub anypoint_client_id: Option<String>,
    pub anypoint_client_secret: Option<String>,
    pub anypoint_org_id: Option<String>,
    pub anypoint_env_id: Option<String>,
    pub object_store_id: Option<String>,
    pub auto_create_store: Option<bool>,
    pub disable_object_store: Option<bool>,
    pub object_store_ttl_seconds: Option<i64>,
    pub task_hot_cache_ttl_seconds: Option<i64>,
    pub task_store_timeout_ms: Option<i64>,

    pub agent_card_source: Option<String>,
    pub agent_card_json: Option<String>,
    /// `true` when `agentCardSource = url` and the codegen registered a
    /// Service for the URL. We can't carry the Service through `RawConfig`
    /// (Service is only constructible by the codegen), so the caller passes
    /// this flag in based on whether the URL was supplied.
    pub agent_card_url_registered: bool,
    pub agent_card_name: Option<String>,
    pub agent_card_description: Option<String>,
    pub agent_card_version: Option<String>,
    pub agent_card_icon_url: Option<String>,
    pub agent_card_documentation_url: Option<String>,
    pub agent_card_provider_organization: Option<String>,
    pub agent_card_provider_url: Option<String>,
    pub agent_card_capabilities_streaming: Option<bool>,
    pub agent_card_capabilities_push_notifications: Option<bool>,
    pub agent_card_default_input_modes: Option<String>,
    pub agent_card_default_output_modes: Option<String>,
    pub agent_card_skills_json: Option<String>,
    pub agent_card_security_schemes_json: Option<String>,
    pub agent_card_override_json: Option<String>,
}

impl PolicyConfig {
    pub fn from_raw(raw: RawConfig) -> Result<Self, ConfigError> {
        let consumer_key = nonempty(raw.consumer_key.clone(), ConfigError::MissingConsumerKey)?;
        let consumer_secret = nonempty(
            raw.consumer_secret.clone(),
            ConfigError::MissingConsumerSecret,
        )?;
        let agent_id = nonempty(raw.agent_id.clone(), ConfigError::MissingAgentId)?;
        let public_base_url =
            nonempty(raw.public_base_url.clone(), ConfigError::MissingPublicBaseUrl)?;

        let anypoint_client_id = raw.anypoint_client_id.clone().unwrap_or_default();
        let anypoint_client_secret = raw.anypoint_client_secret.clone().unwrap_or_default();
        let anypoint_org_id =
            nonempty(raw.anypoint_org_id.clone(), ConfigError::MissingAnypointOrgId)?;
        let anypoint_env_id =
            nonempty(raw.anypoint_env_id.clone(), ConfigError::MissingAnypointEnvId)?;
        let object_store_id =
            nonempty(raw.object_store_id.clone(), ConfigError::MissingObjectStoreId)?;
        let auto_create_store = raw.auto_create_store.unwrap_or(true);
        let disable_object_store = raw.disable_object_store.unwrap_or(false);
        let object_store_ttl_seconds =
            clamp_u32(raw.object_store_ttl_seconds, 60, 2_592_000, 86_400);

        let protocol_version =
            ProtocolVersion::parse(raw.protocol_version.as_deref().unwrap_or("0.3.0"))?;
        let agent_card_source =
            AgentCardSource::parse(raw.agent_card_source.as_deref().unwrap_or("structured"))?;

        let a2a_rpc_path = normalize_path(raw.a2a_rpc_path.as_deref().unwrap_or("/"));
        let strict_mode = raw.strict_mode.unwrap_or(false);
        let bypass_user = raw.bypass_user.unwrap_or(true);
        let cache_safety_margin_seconds = clamp_u32(raw.cache_safety_margin_seconds, 0, 600, 60);
        let task_hot_cache_ttl_seconds = clamp_u32(raw.task_hot_cache_ttl_seconds, 0, 3600, 60);
        let task_store_timeout_ms =
            clamp_u32(raw.task_store_timeout_ms, 100, 30_000, 1_500) as u64;

        // Validate the source-specific inputs and parse the structured card
        // payload regardless of source so a misconfigured field surfaces at
        // policy load rather than on first request.
        let agent_card_json = match agent_card_source {
            AgentCardSource::InlineJson | AgentCardSource::File => {
                let s = raw
                    .agent_card_json
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or(ConfigError::AgentCardJsonEmpty)?;
                let parsed: serde_json::Value =
                    serde_json::from_str(&s).map_err(|e| ConfigError::InvalidJson {
                        field: "agentCardJson",
                        error: e.to_string(),
                    })?;
                if !parsed.is_object() {
                    return Err(ConfigError::AgentCardJsonNotObject);
                }
                Some(s)
            }
            AgentCardSource::Url => {
                if !raw.agent_card_url_registered {
                    return Err(ConfigError::AgentCardUrlNotRegistered);
                }
                None
            }
            AgentCardSource::Structured => None,
        };

        let structured_card = parse_structured(&raw)?;
        let agent_card_override = parse_optional_object(
            raw.agent_card_override_json.as_deref(),
            "agentCardOverrideJson",
            ConfigError::AgentCardOverrideNotObject,
        )?;

        Ok(Self {
            consumer_key,
            consumer_secret,
            agent_id,
            bypass_user,
            cache_safety_margin_seconds,

            protocol_version,
            a2a_rpc_path,
            public_base_url,
            strict_mode,

            anypoint_client_id,
            anypoint_client_secret,
            anypoint_org_id,
            anypoint_env_id,
            object_store_id,
            auto_create_store,
            disable_object_store,
            object_store_ttl_seconds,
            task_hot_cache_ttl_seconds,
            task_store_timeout_ms,

            agent_card_source,
            agent_card_json,
            structured_card,
            agent_card_override,
        })
    }
}

fn parse_structured(raw: &RawConfig) -> Result<StructuredCardConfig, ConfigError> {
    let skills_str = raw
        .agent_card_skills_json
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("[]");
    let skills: serde_json::Value =
        serde_json::from_str(skills_str).map_err(|e| ConfigError::InvalidJson {
            field: "agentCardSkillsJson",
            error: e.to_string(),
        })?;
    if !skills.is_array() {
        return Err(ConfigError::AgentCardSkillsNotArray);
    }

    let security_schemes = parse_optional_object(
        raw.agent_card_security_schemes_json.as_deref(),
        "agentCardSecuritySchemesJson",
        ConfigError::AgentCardSecuritySchemesNotObject,
    )?;

    let provider_organization = nonempty_opt(raw.agent_card_provider_organization.clone());
    let provider_url = nonempty_opt(raw.agent_card_provider_url.clone());
    if provider_organization.is_some() != provider_url.is_some() {
        return Err(ConfigError::AgentCardProviderHalfSet);
    }

    Ok(StructuredCardConfig {
        name: nonempty_opt(raw.agent_card_name.clone()),
        description: nonempty_opt(raw.agent_card_description.clone()),
        version: nonempty_opt(raw.agent_card_version.clone()),
        icon_url: nonempty_opt(raw.agent_card_icon_url.clone()),
        documentation_url: nonempty_opt(raw.agent_card_documentation_url.clone()),
        provider_organization,
        provider_url,
        capabilities_streaming: raw.agent_card_capabilities_streaming.unwrap_or(false),
        capabilities_push_notifications: raw
            .agent_card_capabilities_push_notifications
            .unwrap_or(false),
        default_input_modes: split_csv(raw.agent_card_default_input_modes.as_deref()),
        default_output_modes: split_csv(raw.agent_card_default_output_modes.as_deref()),
        skills,
        security_schemes,
    })
}

fn parse_optional_object(
    s: Option<&str>,
    field: &'static str,
    not_object: ConfigError,
) -> Result<Option<serde_json::Value>, ConfigError> {
    let s = match s.map(str::trim) {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(None),
    };
    let v: serde_json::Value =
        serde_json::from_str(s).map_err(|e| ConfigError::InvalidJson {
            field,
            error: e.to_string(),
        })?;
    if !v.is_object() {
        return Err(not_object);
    }
    Ok(Some(v))
}

fn nonempty(value: Option<String>, err: ConfigError) -> Result<String, ConfigError> {
    match value {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err(err),
    }
}

fn nonempty_opt(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

fn split_csv(s: Option<&str>) -> Vec<String> {
    let s = s.unwrap_or("text/plain");
    let modes: Vec<String> = s
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect();
    if modes.is_empty() {
        vec!["text/plain".into()]
    } else {
        modes
    }
}

fn normalize_path(s: &str) -> String {
    let mut s = s.trim().to_string();
    if s.is_empty() {
        return "/".into();
    }
    if !s.starts_with('/') {
        s = format!("/{s}");
    }
    s
}

fn clamp_u32(v: Option<i64>, min: i64, max: i64, default: i64) -> u32 {
    let v = v.unwrap_or(default).clamp(min, max);
    v as u32
}

/// Deep-merge `overlay` into `base` in place, recursing into nested objects.
/// Arrays and scalars are replaced wholesale, matching the JSON Merge Patch
/// (RFC 7396) intuition that operators expect when overriding individual
/// AgentCard fields.
pub fn deep_merge(base: &mut serde_json::Value, overlay: serde_json::Value) {
    use serde_json::Value;
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => deep_merge(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, overlay) => *slot = overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> RawConfig {
        RawConfig {
            consumer_key: Some("client-id".into()),
            consumer_secret: Some("client-secret".into()),
            agent_id: Some("0XXxx0000000000".into()),
            public_base_url: Some("https://gw.example.com/a2a".into()),
            anypoint_org_id: Some("org-1".into()),
            anypoint_env_id: Some("env-1".into()),
            object_store_id: Some("a2a-tasks".into()),
            agent_card_source: Some("structured".into()),
            agent_card_name: Some("Test Agent".into()),
            agent_card_description: Some("desc".into()),
            ..Default::default()
        }
    }

    #[test]
    fn applies_defaults() {
        let cfg = PolicyConfig::from_raw(minimal()).unwrap();
        assert_eq!(cfg.protocol_version, ProtocolVersion::V0_3_0);
        assert_eq!(cfg.a2a_rpc_path, "/");
        assert!(!cfg.strict_mode);
        assert!(cfg.bypass_user);
        assert_eq!(cfg.cache_safety_margin_seconds, 60);
        assert_eq!(cfg.task_hot_cache_ttl_seconds, 60);
        assert_eq!(cfg.task_store_timeout_ms, 1500);
        assert_eq!(cfg.agent_card_source, AgentCardSource::Structured);
        assert_eq!(cfg.structured_card.default_input_modes, vec!["text/plain"]);
    }

    #[test]
    fn rejects_unknown_protocol_version() {
        let mut raw = minimal();
        raw.protocol_version = Some("9.9.9".into());
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::UnsupportedProtocolVersion(_))
        ));
    }

    #[test]
    fn rejects_unknown_agent_card_source() {
        let mut raw = minimal();
        raw.agent_card_source = Some("smoke-signal".into());
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::UnsupportedAgentCardSource(_))
        ));
    }

    #[test]
    fn rejects_inline_json_when_blank() {
        let mut raw = minimal();
        raw.agent_card_source = Some("inline_json".into());
        raw.agent_card_json = Some("   ".into());
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::AgentCardJsonEmpty)
        ));
    }

    #[test]
    fn rejects_inline_json_when_not_object() {
        let mut raw = minimal();
        raw.agent_card_source = Some("inline_json".into());
        raw.agent_card_json = Some("[]".into());
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::AgentCardJsonNotObject)
        ));
    }

    #[test]
    fn rejects_url_when_no_service_registered() {
        let mut raw = minimal();
        raw.agent_card_source = Some("url".into());
        raw.agent_card_url_registered = false;
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::AgentCardUrlNotRegistered)
        ));
    }

    #[test]
    fn accepts_url_when_service_registered() {
        let mut raw = minimal();
        raw.agent_card_source = Some("url".into());
        raw.agent_card_url_registered = true;
        let cfg = PolicyConfig::from_raw(raw).unwrap();
        assert_eq!(cfg.agent_card_source, AgentCardSource::Url);
        assert!(cfg.agent_card_json.is_none());
    }

    #[test]
    fn rejects_skills_when_not_array() {
        let mut raw = minimal();
        raw.agent_card_skills_json = Some("{}".into());
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::AgentCardSkillsNotArray)
        ));
    }

    #[test]
    fn rejects_invalid_skills_json() {
        let mut raw = minimal();
        raw.agent_card_skills_json = Some("not-json".into());
        let err = PolicyConfig::from_raw(raw).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidJson { field: "agentCardSkillsJson", .. }));
    }

    #[test]
    fn parses_structured_capabilities_and_modes() {
        let mut raw = minimal();
        raw.agent_card_capabilities_streaming = Some(true);
        raw.agent_card_default_input_modes = Some("text/plain, application/json".into());
        raw.agent_card_skills_json =
            Some(r#"[{"id":"s1","name":"Greet","description":"d","tags":["t"]}]"#.into());
        let cfg = PolicyConfig::from_raw(raw).unwrap();
        assert!(cfg.structured_card.capabilities_streaming);
        assert_eq!(
            cfg.structured_card.default_input_modes,
            vec!["text/plain".to_string(), "application/json".to_string()]
        );
        assert!(cfg.structured_card.skills.is_array());
    }

    #[test]
    fn provider_org_without_url_is_rejected() {
        let mut raw = minimal();
        raw.agent_card_provider_organization = Some("Acme".into());
        raw.agent_card_provider_url = None;
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::AgentCardProviderHalfSet)
        ));
    }

    #[test]
    fn provider_url_without_org_is_rejected() {
        let mut raw = minimal();
        raw.agent_card_provider_organization = None;
        raw.agent_card_provider_url = Some("https://acme.example.com".into());
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::AgentCardProviderHalfSet)
        ));
    }

    #[test]
    fn provider_both_or_neither_is_accepted() {
        let mut raw = minimal();
        raw.agent_card_provider_organization = Some("Acme".into());
        raw.agent_card_provider_url = Some("https://acme.example.com".into());
        assert!(PolicyConfig::from_raw(raw.clone()).is_ok());

        let mut raw = minimal();
        raw.agent_card_provider_organization = None;
        raw.agent_card_provider_url = None;
        assert!(PolicyConfig::from_raw(raw).is_ok());
    }

    #[test]
    fn override_must_be_object() {
        let mut raw = minimal();
        raw.agent_card_override_json = Some("[1,2,3]".into());
        assert!(matches!(
            PolicyConfig::from_raw(raw),
            Err(ConfigError::AgentCardOverrideNotObject)
        ));
    }

    #[test]
    fn override_when_object_is_kept() {
        let mut raw = minimal();
        raw.agent_card_override_json = Some(r#"{"name":"Override"}"#.into());
        let cfg = PolicyConfig::from_raw(raw).unwrap();
        assert!(cfg.agent_card_override.is_some());
    }

    #[test]
    fn deep_merge_overrides_nested_field() {
        let mut base = serde_json::json!({
            "name": "old",
            "capabilities": { "streaming": false, "pushNotifications": true }
        });
        let overlay = serde_json::json!({
            "capabilities": { "streaming": true }
        });
        deep_merge(&mut base, overlay);
        assert_eq!(base["capabilities"]["streaming"], true);
        assert_eq!(base["capabilities"]["pushNotifications"], true);
        assert_eq!(base["name"], "old");
    }

    #[test]
    fn deep_merge_replaces_arrays_wholesale() {
        let mut base = serde_json::json!({ "tags": ["a", "b"] });
        let overlay = serde_json::json!({ "tags": ["c"] });
        deep_merge(&mut base, overlay);
        assert_eq!(base["tags"], serde_json::json!(["c"]));
    }

    #[test]
    fn normalize_path_adds_leading_slash() {
        assert_eq!(normalize_path("a2a"), "/a2a");
        assert_eq!(normalize_path("/a2a"), "/a2a");
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn clamp_u32_respects_bounds() {
        assert_eq!(clamp_u32(Some(-5), 0, 100, 10), 0);
        assert_eq!(clamp_u32(Some(200), 0, 100, 10), 100);
        assert_eq!(clamp_u32(None, 0, 100, 10), 10);
    }
}
