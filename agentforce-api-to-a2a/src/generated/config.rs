use serde::Deserialize;
#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(alias = "a2aRpcPath")]
    pub a_2_a_rpc_path: Option<String>,
    #[serde(alias = "agentCardCapabilitiesPushNotifications")]
    pub agent_card_capabilities_push_notifications: Option<bool>,
    #[serde(alias = "agentCardCapabilitiesStreaming")]
    pub agent_card_capabilities_streaming: Option<bool>,
    #[serde(alias = "agentCardDefaultInputModes")]
    pub agent_card_default_input_modes: Option<String>,
    #[serde(alias = "agentCardDefaultOutputModes")]
    pub agent_card_default_output_modes: Option<String>,
    #[serde(alias = "agentCardDescription")]
    pub agent_card_description: Option<String>,
    #[serde(alias = "agentCardDocumentationUrl")]
    pub agent_card_documentation_url: Option<String>,
    #[serde(alias = "agentCardIconUrl")]
    pub agent_card_icon_url: Option<String>,
    #[serde(alias = "agentCardJson")]
    pub agent_card_json: Option<String>,
    #[serde(alias = "agentCardName")]
    pub agent_card_name: Option<String>,
    #[serde(alias = "agentCardOverrideJson")]
    pub agent_card_override_json: Option<String>,
    #[serde(alias = "agentCardProviderOrganization")]
    pub agent_card_provider_organization: Option<String>,
    #[serde(alias = "agentCardProviderUrl")]
    pub agent_card_provider_url: Option<String>,
    #[serde(alias = "agentCardSecuritySchemesJson")]
    pub agent_card_security_schemes_json: Option<String>,
    #[serde(alias = "agentCardSkillsJson")]
    pub agent_card_skills_json: Option<String>,
    #[serde(alias = "agentCardSource")]
    pub agent_card_source: Option<String>,
    #[serde(
        alias = "agentCardUrl",
        default,
        deserialize_with = "pdk::serde::deserialize_service_opt"
    )]
    pub agent_card_url: Option<pdk::hl::Service>,
    #[serde(alias = "agentCardVersion")]
    pub agent_card_version: Option<String>,
    #[serde(alias = "agentId")]
    pub agent_id: String,
    #[serde(
        alias = "agentforceApiUrl",
        deserialize_with = "pdk::serde::deserialize_service"
    )]
    pub agentforce_api_url: pdk::hl::Service,
    #[serde(alias = "anypointClientId")]
    pub anypoint_client_id: String,
    #[serde(alias = "anypointClientSecret")]
    pub anypoint_client_secret: String,
    #[serde(alias = "anypointEnvId")]
    pub anypoint_env_id: String,
    #[serde(alias = "anypointOrgId")]
    pub anypoint_org_id: String,
    #[serde(
        alias = "anypointTokenUrl",
        deserialize_with = "pdk::serde::deserialize_service"
    )]
    pub anypoint_token_url: pdk::hl::Service,
    #[serde(alias = "autoCreateStore")]
    pub auto_create_store: Option<bool>,
    #[serde(alias = "bypassUser")]
    pub bypass_user: Option<bool>,
    #[serde(alias = "cacheSafetyMarginSeconds")]
    pub cache_safety_margin_seconds: Option<i64>,
    #[serde(alias = "consumerKey")]
    pub consumer_key: String,
    #[serde(alias = "consumerSecret")]
    pub consumer_secret: String,
    #[serde(alias = "disableObjectStore")]
    pub disable_object_store: Option<bool>,
    #[serde(alias = "myDomainUrl", deserialize_with = "pdk::serde::deserialize_service")]
    pub my_domain_url: pdk::hl::Service,
    #[serde(
        alias = "objectStoreBaseUrl",
        deserialize_with = "pdk::serde::deserialize_service"
    )]
    pub object_store_base_url: pdk::hl::Service,
    #[serde(alias = "objectStoreId")]
    pub object_store_id: String,
    #[serde(alias = "objectStoreTtlSeconds")]
    pub object_store_ttl_seconds: Option<i64>,
    #[serde(alias = "protocolVersion")]
    pub protocol_version: Option<String>,
    #[serde(alias = "publicBaseUrl")]
    pub public_base_url: String,
    #[serde(alias = "strictMode")]
    pub strict_mode: Option<bool>,
    #[serde(alias = "taskHotCacheTtlSeconds")]
    pub task_hot_cache_ttl_seconds: Option<i64>,
    #[serde(alias = "taskStoreTimeoutMs")]
    pub task_store_timeout_ms: Option<i64>,
}
#[pdk::hl::entrypoint_flex]
fn init(abi: &dyn pdk::flex_abi::api::FlexAbi) -> Result<(), anyhow::Error> {
    let config: Config = serde_json::from_slice(abi.get_configuration())
        .map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse configuration '{}'. Cause: {}",
                String::from_utf8_lossy(abi.get_configuration()), err
            )
        })?;
    if config.agent_card_url.is_some() {
        let service = config.agent_card_url.unwrap();
        abi.service_create(service)?;
    }
    abi.service_create(config.agentforce_api_url)?;
    abi.service_create(config.anypoint_token_url)?;
    abi.service_create(config.my_domain_url)?;
    abi.service_create(config.object_store_base_url)?;
    abi.setup()?;
    Ok(())
}
