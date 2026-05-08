//! Agentforce API -> A2A 0.3.0 server policy.
//!
//! On request:
//!
//!   1. Classify the path (`agent-card`, `a2a-rpc`, or passthrough).
//!   2. agent-card: short-circuit with the cached/fetched AgentCard JSON.
//!   3. a2a-rpc:    read the body, parse JSON-RPC, dispatch to
//!                  `message/send` | `tasks/get` | `tasks/cancel`,
//!                  short-circuit with the JSON-RPC response.
//!   4. passthrough: continue (or 404 if `strictMode = true`).

mod a2a;
mod agent_card;
mod agentforce;
mod cache;
mod config;
mod generated;
mod jsonrpc;
mod router;
mod store;

use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use pdk::cache::{Cache, CacheBuilder};
use pdk::hl::*;
use pdk::logger;

use crate::a2a::methods::Dispatcher;
use crate::agent_card::CardProvider;
use crate::agentforce::auth::{AgentforceAuth, AgentforceAuthConfig};
use crate::agentforce::client::AgentforceClient;
use crate::config::{PolicyConfig, RawConfig};
use crate::generated::config::Config;
use crate::jsonrpc::{JsonRpcError, INVALID_REQUEST};
use crate::router::{classify, Route};
use crate::store::object_store_v2::{OS2Config, ObjectStoreV2};
use crate::store::task_store::TaskStore;

/// Cache id for the worker-shared PDK cache. Stores OAuth tokens
/// (Salesforce + Anypoint), the URL-fetched AgentCard, and the hot layer
/// of the TaskStore.
const CACHE_ID: &str = "agentforce-a2a-shared";

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// ISO-8601 UTC timestamp for the current moment. Best-effort: if the
/// host clock is broken we still produce something serializable so we
/// never fail an RPC over a missing timestamp.
fn now_iso() -> String {
    let secs = now_unix();
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Tiny, dependency-free UNIX-seconds -> Y/M/D/h/m/s converter (UTC).
/// Avoids pulling `chrono` into the wasm binary.
fn unix_to_ymdhms(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let day_secs = 86_400;
    let mut days = (secs / day_secs) as i64;
    let secs_of_day = (secs % day_secs) as u32;
    let hour = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    let sec = secs_of_day % 60;

    // 1970-01-01 was a Thursday; not needed here. Compute calendar from
    // days since epoch.
    let mut year = 1970i64;
    loop {
        let leap = is_leap(year);
        let dy = if leap { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let mdays = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 0;
    for (i, m) in mdays.iter().enumerate() {
        if days < *m {
            month = i + 1;
            break;
        }
        days -= *m;
    }
    let day = days as u32 + 1;
    (year as u32, month as u32, day, hour, min, sec)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

/// Bridge codegen-typed `Config` into the host-testable `RawConfig`.
/// Service-typed properties (`myDomainUrl`, etc.) stay on `Config` and
/// are consumed directly in `configure`.
impl From<&Config> for RawConfig {
    fn from(c: &Config) -> Self {
        RawConfig {
            consumer_key: Some(c.consumer_key.clone()),
            consumer_secret: Some(c.consumer_secret.clone()),
            agent_id: Some(c.agent_id.clone()),
            bypass_user: c.bypass_user,
            cache_safety_margin_seconds: c.cache_safety_margin_seconds,
            protocol_version: c.protocol_version.clone(),
            a2a_rpc_path: c.a2a_rpc_path.clone(),
            public_base_url: Some(c.public_base_url.clone()),
            strict_mode: c.strict_mode,
            anypoint_client_id: Some(c.anypoint_client_id.clone()),
            anypoint_client_secret: Some(c.anypoint_client_secret.clone()),
            anypoint_org_id: Some(c.anypoint_org_id.clone()),
            anypoint_env_id: Some(c.anypoint_env_id.clone()),
            object_store_id: Some(c.object_store_id.clone()),
            task_hot_cache_ttl_seconds: c.task_hot_cache_ttl_seconds,
            task_store_timeout_ms: c.task_store_timeout_ms,
            agent_card_source: c.agent_card_source.clone(),
            agent_card_json: c.agent_card_json.clone(),
            agent_card_url_registered: c.agent_card_url.is_some(),
            agent_card_name: c.agent_card_name.clone(),
            agent_card_description: c.agent_card_description.clone(),
            agent_card_version: c.agent_card_version.clone(),
            agent_card_icon_url: c.agent_card_icon_url.clone(),
            agent_card_documentation_url: c.agent_card_documentation_url.clone(),
            agent_card_provider_organization: c.agent_card_provider_organization.clone(),
            agent_card_provider_url: c.agent_card_provider_url.clone(),
            agent_card_capabilities_streaming: c.agent_card_capabilities_streaming,
            agent_card_capabilities_push_notifications: c.agent_card_capabilities_push_notifications,
            agent_card_default_input_modes: c.agent_card_default_input_modes.clone(),
            agent_card_default_output_modes: c.agent_card_default_output_modes.clone(),
            agent_card_skills_json: c.agent_card_skills_json.clone(),
            agent_card_security_schemes_json: c.agent_card_security_schemes_json.clone(),
            agent_card_override_json: c.agent_card_override_json.clone(),
        }
    }
}

#[derive(Clone)]
struct PolicyState {
    cfg: Rc<PolicyConfig>,
    card: Rc<CardProvider>,
    dispatcher: Rc<Dispatcher>,
}

async fn request_filter(
    request: RequestHeadersState,
    state: PolicyState,
    client: HttpClient,
) -> Flow<()> {
    let method = request.method();
    let path = request.path();
    let route = classify(&method, &path, &state.cfg.a2a_rpc_path);

    match route {
        Route::AgentCard => match state.card.bytes(&client, now_unix()).await {
            Ok(bytes) => Flow::Break(make_json_response(200, bytes.as_slice())),
            Err(e) => {
                logger::error!("agent-card: failed to provide card: {e}");
                Flow::Break(make_json_response(
                    500,
                    br#"{"error":"agent_card_unavailable"}"#,
                ))
            }
        },
        Route::A2aRpc => handle_rpc(request, state, client).await,
        Route::Passthrough => {
            if state.cfg.strict_mode {
                Flow::Break(make_json_response(
                    404,
                    br#"{"error":"not_found","reason":"strictMode"}"#,
                ))
            } else {
                Flow::Continue(())
            }
        }
    }
}

async fn handle_rpc(
    request: RequestHeadersState,
    state: PolicyState,
    client: HttpClient,
) -> Flow<()> {
    if !request.contains_body() {
        let err = JsonRpcError::new(None, INVALID_REQUEST, "Invalid Request: empty body");
        return Flow::Break(make_json_response(200, err.into_bytes().as_slice()));
    }

    let body_state = request.into_body_state().await;
    let body = body_state.handler().body();

    let parsed = match jsonrpc::parse_request(&body) {
        Ok(r) => r,
        Err(err) => {
            return Flow::Break(make_json_response(200, err.into_bytes().as_slice()));
        }
    };

    let result = state
        .dispatcher
        .dispatch(&client, parsed, now_unix(), now_iso())
        .await;

    let bytes = match result {
        Ok(success) => success.into_bytes(),
        Err(err) => err.into_bytes(),
    };
    Flow::Break(make_json_response(200, &bytes))
}

fn make_json_response(status: u32, body: &[u8]) -> Response {
    Response::new(status)
        .with_headers(vec![(
            "content-type".to_string(),
            "application/json".to_string(),
        )])
        .with_body(body.to_vec())
}

#[entrypoint]
pub async fn configure(
    launcher: Launcher,
    Configuration(bytes): Configuration,
    cache_builder: CacheBuilder,
) -> anyhow::Result<()> {
    let raw: Config = serde_json::from_slice(&bytes)
        .map_err(|e| anyhow!("invalid policy configuration: {e}"))?;

    let cfg = PolicyConfig::from_raw((&raw).into())
        .map_err(|e| anyhow!("policy configuration rejected: {e}"))?;
    let cfg = Rc::new(cfg);

    let cache: Rc<dyn Cache> = Rc::from(cache_builder.new(CACHE_ID.to_string()).shared().build());

    // Extract upstream Services. These are already registered as Envoy
    // upstream clusters by `init` in `generated::config`.
    let my_domain_service = Rc::new(raw.my_domain_url.clone());
    let agentforce_api_service = Rc::new(raw.agentforce_api_url.clone());
    let object_store_service = Rc::new(raw.object_store_base_url.clone());
    let anypoint_token_service = Rc::new(raw.anypoint_token_url.clone());
    let agent_card_url_service = raw.agent_card_url.clone().map(Rc::new);

    let my_domain_authority = my_domain_service.uri().authority().to_string();
    let my_domain_scheme = my_domain_service.uri().scheme().to_string();
    let my_domain_url_value = format!("{my_domain_scheme}://{my_domain_authority}");
    let anypoint_token_authority = anypoint_token_service.uri().authority().to_string();

    // Salesforce/Agentforce auth.
    let auth = Rc::new(AgentforceAuth::new(
        AgentforceAuthConfig {
            consumer_key: cfg.consumer_key.clone(),
            consumer_secret: cfg.consumer_secret.clone(),
            my_domain_url_for_cache_key: my_domain_authority.clone(),
            cache_safety_margin_seconds: cfg.cache_safety_margin_seconds,
        },
        cache.clone(),
        my_domain_service.clone(),
    ));

    // Agentforce client.
    let agentforce_client = Rc::new(AgentforceClient::new(
        auth.clone(),
        agentforce_api_service.clone(),
        my_domain_url_value,
        cfg.agent_id.clone(),
        cfg.bypass_user,
    ));

    // Object Store v2 + TaskStore.
    let os2 = Rc::new(ObjectStoreV2::new(
        OS2Config {
            anypoint_client_id: cfg.anypoint_client_id.clone(),
            anypoint_client_secret: cfg.anypoint_client_secret.clone(),
            anypoint_org_id: cfg.anypoint_org_id.clone(),
            anypoint_env_id: cfg.anypoint_env_id.clone(),
            object_store_id: cfg.object_store_id.clone(),
            anypoint_token_url_for_cache_key: anypoint_token_authority.clone(),
            cache_safety_margin_seconds: cfg.cache_safety_margin_seconds,
            timeout_ms: cfg.task_store_timeout_ms,
        },
        cache.clone(),
        anypoint_token_service.clone(),
        object_store_service.clone(),
    ));
    let task_store = Rc::new(TaskStore::new(
        cache.clone(),
        os2.clone(),
        cfg.task_hot_cache_ttl_seconds,
    ));

    let dispatcher = Rc::new(Dispatcher {
        client: agentforce_client.clone(),
        store: task_store.clone(),
    });

    // Agent card.
    let card = Rc::new(CardProvider::new(
        cfg.clone(),
        cache.clone(),
        agent_card_url_service.clone(),
    ));
    if let Err(e) = card.warm() {
        return Err(anyhow!("agent card configuration rejected: {e}"));
    }

    let state = PolicyState {
        cfg: cfg.clone(),
        card,
        dispatcher,
    };

    let filter = on_request(move |request, client: HttpClient| {
        let state = state.clone();
        async move { request_filter(request, state, client).await }
    });

    launcher.launch(filter).await?;
    Ok(())
}
