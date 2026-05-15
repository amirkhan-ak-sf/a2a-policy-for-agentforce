//! Publish a resolved agent-card as a new version of the configured
//! Exchange `agent` asset.
//!
//! Wire shape (per the user-pinned curl):
//!   POST <exchangeBaseUrl>/exchange/api/v2/organizations/{orgId}/assets/{groupId}/{assetId}/{version}
//!   Authorization: bearer <token>
//!   x-sync-publication: true
//!   Content-Type: multipart/form-data; boundary=...
//!     name=<asset name>
//!     description=<description>
//!     type=agent
//!     properties.platform=mulesoft
//!     files.agent-metadata.json=<agent-card.json bytes>
//!
//! Treats 2xx as success and 409 as idempotent re-apply. Anything else
//! is logged at warn and the caller continues.

use std::time::Duration;

use pdk::cache::Cache;
use pdk::hl::{HttpClient, Service};
use pdk::logger;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::cache::{token_cache_key, CachedToken};
use crate::config::{ExchangePublishConfig, PolicyConfig};
use crate::exchange::multipart::{File, MultipartBuilder};

/// Cache key prefix for the Anypoint platform OAuth token used to publish
/// to Exchange. Distinct from any other token caches the policy might use.
const EXCHANGE_TOKEN_PREFIX: &str = "anypoint-exchange";

/// Wall-clock cap on the Exchange POST. Sync publish of an `agent` asset
/// is fast (1-2s per the captured UI traffic). Tighten so a slow Exchange
/// never stalls request handling.
const PUBLISH_TIMEOUT_SECS: u64 = 10;

/// Wall-clock cap on the OAuth token call.
const TOKEN_TIMEOUT_SECS: u64 = 5;

/// Default OAuth token lifetime when the IdP doesn't return `expires_in`.
const DEFAULT_EXPIRES_IN_SECS: u64 = 1800;

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("token transport error: {0}")]
    TokenTransport(String),

    #[error("token endpoint returned HTTP {status}: {body}")]
    TokenHttpStatus { status: u32, body: String },

    #[error("token response was not valid JSON: {0}")]
    TokenBadJson(String),

    #[error("token response missing access_token")]
    TokenMissingAccessToken,

    #[error("publish transport error: {0}")]
    PublishTransport(String),

    #[error("publish returned HTTP {status}: {body}")]
    PublishHttpStatus { status: u32, body: String },

    #[error("agent-card bytes were not a JSON object")]
    BadCard,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}
fn default_expires_in() -> u64 {
    DEFAULT_EXPIRES_IN_SECS
}

pub struct PublishContext<'a> {
    pub cfg: &'a PolicyConfig,
    pub publish_cfg: &'a ExchangePublishConfig,
    pub cache: &'a dyn Cache,
    pub anypoint_token_service: &'a Service,
    pub exchange_service: &'a Service,
    /// Authority of the token URL (host[:port]) used as the cache-key salt.
    pub anypoint_token_authority: &'a str,
    pub now_unix: u64,
}

/// Mint or fetch a cached Anypoint platform bearer token.
async fn get_token(
    client: &HttpClient,
    ctx: &PublishContext<'_>,
) -> Result<String, PublishError> {
    let cache_key = token_cache_key(
        EXCHANGE_TOKEN_PREFIX,
        &ctx.publish_cfg.anypoint_client_id,
        ctx.anypoint_token_authority,
    );

    if let Some(bytes) = ctx.cache.get(&cache_key) {
        if let Ok(entry) = serde_json::from_slice::<CachedToken>(&bytes) {
            if !entry.needs_refresh(ctx.now_unix, ctx.cfg.cache_safety_margin_seconds) {
                return Ok(entry.access_token);
            }
        }
    }

    let body = serde_urlencoded::to_string(&[
        ("grant_type", "client_credentials"),
        ("client_id", ctx.publish_cfg.anypoint_client_id.as_str()),
        (
            "client_secret",
            ctx.publish_cfg.anypoint_client_secret.as_str(),
        ),
    ])
    .expect("urlencoded for &str is infallible");

    let response = client
        .request(ctx.anypoint_token_service)
        .headers(vec![
            ("content-type", "application/x-www-form-urlencoded"),
            ("accept", "application/json"),
        ])
        .body(body.as_bytes())
        .timeout(Duration::from_secs(TOKEN_TIMEOUT_SECS))
        .post()
        .await
        .map_err(|e| PublishError::TokenTransport(e.to_string()))?;

    let status = response.status_code();
    let body_bytes = response.body();
    if !(200..300).contains(&status) {
        let body_str = String::from_utf8_lossy(body_bytes);
        return Err(PublishError::TokenHttpStatus {
            status,
            body: truncate(&body_str, 256),
        });
    }

    let parsed: TokenResponse =
        serde_json::from_slice(body_bytes).map_err(|e| PublishError::TokenBadJson(e.to_string()))?;
    if parsed.access_token.is_empty() {
        return Err(PublishError::TokenMissingAccessToken);
    }

    let entry = CachedToken::new(parsed.access_token.clone(), ctx.now_unix, parsed.expires_in);
    if let Ok(bytes) = serde_json::to_vec(&entry) {
        let _ = ctx.cache.save(&cache_key, bytes);
    }
    Ok(parsed.access_token)
}

/// POST a new asset version. Returns Ok(()) on 2xx or 409; Err on
/// everything else. Caller logs the Err at warn and continues.
pub async fn publish_agent_card(
    client: &HttpClient,
    ctx: PublishContext<'_>,
    card_bytes: &[u8],
) -> Result<(), PublishError> {
    // 1. Mint or fetch token.
    logger::info!(
        "exchange-publish: minting Anypoint bearer for {}/{}",
        ctx.publish_cfg.group_id,
        ctx.publish_cfg.asset_id
    );
    let token = get_token(client, &ctx).await?;

    // 2. Parse the card so we can derive name/description and reject malformed input.
    let card_json: Value = serde_json::from_slice(card_bytes).map_err(|_| PublishError::BadCard)?;
    if !card_json.is_object() {
        return Err(PublishError::BadCard);
    }

    let card_name = card_json
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| ctx.publish_cfg.asset_id.clone());
    let card_description = card_json
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);

    // 3. Pick the initial version. Operator supplies the base via
    //    `agentCardVersion` (default "1.0.0"); we POST that first and, on
    //    409 (already exists), fetch the existing versions and bump the
    //    patch.
    let base_version = ctx
        .cfg
        .structured_card
        .version
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("1.0.0")
        .to_string();
    let _ = card_description; // not accepted by Exchange agent type — keep var to avoid unused warnings
    let bearer = format!("Bearer {token}");

    // 4. First attempt at base_version.
    let outcome = post_version(client, &ctx, &bearer, &card_name, &base_version, card_bytes).await?;
    match outcome {
        PostOutcome::Created => return Ok(()),
        PostOutcome::AlreadyExists => {
            logger::info!(
                "exchange-publish: v{base_version} already exists for {}/{}; auto-bumping patch",
                ctx.publish_cfg.group_id,
                ctx.publish_cfg.asset_id
            );
        }
    }

    // 5. Auto-bump path: fetch existing versions in this minor stream and
    //    bump to next free patch.
    let next_version = match next_free_patch(client, &ctx, &bearer, &base_version).await {
        Ok(v) => v,
        Err(e) => {
            logger::warn!(
                "exchange-publish: failed to enumerate existing versions: {e}; giving up"
            );
            return Ok(());
        }
    };

    logger::info!(
        "exchange-publish: retrying with bumped version {next_version}"
    );
    let outcome = post_version(client, &ctx, &bearer, &card_name, &next_version, card_bytes).await?;
    match outcome {
        PostOutcome::Created => Ok(()),
        PostOutcome::AlreadyExists => {
            logger::warn!(
                "exchange-publish: bumped v{next_version} also reported already-exists; giving up"
            );
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PostOutcome {
    Created,
    AlreadyExists,
}

/// POST a single version. Returns `Created` on 2xx, `AlreadyExists` on 409
/// (regardless of code field), and Err on anything else.
async fn post_version(
    client: &HttpClient,
    ctx: &PublishContext<'_>,
    bearer: &str,
    asset_name: &str,
    version: &str,
    card_bytes: &[u8],
) -> Result<PostOutcome, PublishError> {
    // Wire shape pinned to the working Exchange UI request:
    //   name=<asset name>
    //   type=agent
    //   status=published
    //   properties.protocol=a2a
    //   properties.platform=agentforce
    //   files.a2a-card.json=<agent-card bytes>
    //
    // The `agent-metadata` classifier triggers strict schema validation
    // that rejects A2A AgentCard fields. The `a2a-card` classifier stores
    // the bytes opaquely (Exchange tags them with
    // `application/a2a-card+json`) and is what the UI itself uses.
    let suffix = sha256_short(card_bytes);
    let boundary = format!("a2a-policy-{}", suffix);
    let mut mp = MultipartBuilder::new(boundary);
    mp.add_text("name", asset_name);
    mp.add_text("type", "agent");
    mp.add_text("status", "published");
    mp.add_text("properties.protocol", "a2a");
    mp.add_text("properties.platform", "agentforce");
    mp.add_file(
        "files.a2a-card.json",
        File {
            filename: "agent-card.json",
            content_type: "application/json",
            bytes: card_bytes,
        },
    );
    let (content_type, body) = mp.finish();

    let path = format!(
        "/exchange/api/v2/organizations/{}/assets/{}/{}/{}",
        urlencoding::encode(&ctx.publish_cfg.anypoint_org_id),
        urlencoding::encode(&ctx.publish_cfg.group_id),
        urlencoding::encode(&ctx.publish_cfg.asset_id),
        urlencoding::encode(version),
    );

    logger::info!(
        "exchange-publish: POST {path} (body={} bytes)",
        body.len()
    );

    let response = client
        .request(ctx.exchange_service)
        .path(&path)
        .headers(vec![
            ("authorization", bearer),
            ("content-type", content_type.as_str()),
            ("accept", "application/json"),
            ("x-sync-publication", "true"),
        ])
        .body(&body)
        .timeout(Duration::from_secs(PUBLISH_TIMEOUT_SECS))
        .post()
        .await
        .map_err(|e| PublishError::PublishTransport(e.to_string()))?;

    let status = response.status_code();
    let body_str = String::from_utf8_lossy(response.body()).to_string();

    if status == 409 {
        logger::info!(
            "exchange-publish: v{version} already exists for {}/{} (409)",
            ctx.publish_cfg.group_id,
            ctx.publish_cfg.asset_id
        );
        Ok(PostOutcome::AlreadyExists)
    } else if (200..300).contains(&status) {
        logger::info!(
            "exchange-publish: posted v{version} to {}/{} ({status})",
            ctx.publish_cfg.group_id,
            ctx.publish_cfg.asset_id
        );
        Ok(PostOutcome::Created)
    } else {
        Err(PublishError::PublishHttpStatus {
            status,
            body: truncate(&body_str, 512),
        })
    }
}

/// GET the asset's existing minor-version stream and pick the next free
/// patch above any returned version. Falls back to bumping the operator's
/// patch by 1 if the response is unparseable.
async fn next_free_patch(
    client: &HttpClient,
    ctx: &PublishContext<'_>,
    bearer: &str,
    base_version: &str,
) -> Result<String, PublishError> {
    let (major, minor, patch) = match parse_semver(base_version) {
        Some(v) => v,
        None => return Ok(format!("{base_version}-1")),
    };
    let minor_stream = format!("{major}.{minor}");

    let path = format!(
        "/exchange/api/v2/assets/{}/{}/minorVersions/{}?status=development&status=published&status=deprecated&strict=false",
        urlencoding::encode(&ctx.publish_cfg.group_id),
        urlencoding::encode(&ctx.publish_cfg.asset_id),
        urlencoding::encode(&minor_stream),
    );

    logger::info!("exchange-publish: GET {path} (enumerate versions)");

    let response = client
        .request(ctx.exchange_service)
        .path(&path)
        .headers(vec![
            ("authorization", bearer),
            ("accept", "application/json"),
        ])
        .timeout(Duration::from_secs(PUBLISH_TIMEOUT_SECS))
        .get()
        .await
        .map_err(|e| PublishError::PublishTransport(e.to_string()))?;

    let status = response.status_code();
    if !(200..300).contains(&status) {
        let body_str = String::from_utf8_lossy(response.body());
        return Err(PublishError::PublishHttpStatus {
            status,
            body: truncate(&body_str, 256),
        });
    }

    let v: Value = serde_json::from_slice(response.body())
        .map_err(|e| PublishError::PublishHttpStatus {
            status: 0,
            body: format!("minorVersions response not JSON: {e}"),
        })?;

    // Collect every patch number in the same major.minor stream from
    // both `version` and the `versions[]` / `otherVersions[]` arrays.
    let mut max_patch = patch;
    collect_patches(&v, major, minor, &mut max_patch);
    if let Some(arr) = v.get("versions").and_then(Value::as_array) {
        for item in arr {
            collect_patches(item, major, minor, &mut max_patch);
        }
    }
    if let Some(arr) = v.get("otherVersions").and_then(Value::as_array) {
        for item in arr {
            collect_patches(item, major, minor, &mut max_patch);
        }
    }

    Ok(format!("{major}.{minor}.{}", max_patch + 1))
}

/// Look at one node's `version` field and lift `max_patch` if it's in the
/// same major.minor stream and higher than the current max.
fn collect_patches(node: &Value, major: u32, minor: u32, max_patch: &mut u32) {
    if let Some(s) = node.get("version").and_then(Value::as_str) {
        if let Some((m, n, p)) = parse_semver(s) {
            if m == major && n == minor && p > *max_patch {
                *max_patch = p;
            }
        }
    }
}

/// Parse a strict `<u32>.<u32>.<u32>` semver. Returns `None` for anything
/// that doesn't match (pre-release suffixes, build metadata, etc.).
fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let major = parts[0].parse::<u32>().ok()?;
    let minor = parts[1].parse::<u32>().ok()?;
    let patch = parts[2].parse::<u32>().ok()?;
    Some((major, minor, patch))
}

fn sha256_short(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let hex = format!("{digest:x}");
    hex[..8].to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_short_is_deterministic() {
        let a = sha256_short(br#"{"name":"x"}"#);
        let b = sha256_short(br#"{"name":"x"}"#);
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
    }

    #[test]
    fn sha256_short_changes_with_content() {
        assert_ne!(sha256_short(b"a"), sha256_short(b"b"));
    }

    #[test]
    fn token_response_parses_with_default_expires_in() {
        let r: TokenResponse = serde_json::from_slice(br#"{"access_token":"abc"}"#).unwrap();
        assert_eq!(r.access_token, "abc");
        assert_eq!(r.expires_in, DEFAULT_EXPIRES_IN_SECS);
    }

    #[test]
    fn parse_semver_accepts_plain_triplet() {
        assert_eq!(parse_semver("1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_semver("12.34.56"), Some((12, 34, 56)));
    }

    #[test]
    fn parse_semver_rejects_non_strict() {
        assert_eq!(parse_semver("1.0"), None);
        assert_eq!(parse_semver("1.0.0-rc1"), None);
        assert_eq!(parse_semver("v1.0.0"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn collect_patches_finds_max_in_same_minor() {
        let v = serde_json::json!({"version": "1.0.5"});
        let mut max = 0u32;
        collect_patches(&v, 1, 0, &mut max);
        assert_eq!(max, 5);
    }

    #[test]
    fn collect_patches_ignores_other_minors() {
        let v = serde_json::json!({"version": "1.1.5"});
        let mut max = 0u32;
        collect_patches(&v, 1, 0, &mut max);
        assert_eq!(max, 0);
    }
}
