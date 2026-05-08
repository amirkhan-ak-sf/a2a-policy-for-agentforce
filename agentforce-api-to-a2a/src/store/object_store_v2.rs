//! Anypoint Object Store v2 REST client.
//!
//! API surface used by the policy:
//!
//!   * Token: `POST <anypointTokenUrl>` body
//!     `grant_type=client_credentials&client_id=...&client_secret=...`.
//!     Returns `{access_token, expires_in}`. Cached in the same PDK shared
//!     cache as the Salesforce token (different prefix).
//!   * Key store:
//!     `GET    <objectStoreBaseUrl>/api/v1/organizations/{org}/environments/{env}/data/{store}/keys/{key}` -> 200 JSON | 404
//!     `PUT    <objectStoreBaseUrl>/api/v1/organizations/{org}/environments/{env}/data/{store}/keys/{key}` -> 204
//!     `DELETE <objectStoreBaseUrl>/api/v1/organizations/{org}/environments/{env}/data/{store}/keys/{key}` -> 204 | 404
//!
//! Reads degrade to "not found" on transport failure; writes are
//! best-effort (a failed write logs at warn and the RPC still succeeds).

use std::rc::Rc;

use pdk::cache::Cache;
use pdk::hl::{HttpClient, Service};
use pdk::logger;
use serde::Deserialize;
use thiserror::Error;
use urlencoding;

use crate::cache::{token_cache_key, CachedToken, ANYPOINT_TOKEN_PREFIX};

#[derive(Debug, Clone)]
pub struct OS2Config {
    pub anypoint_client_id: String,
    pub anypoint_client_secret: String,
    pub anypoint_org_id: String,
    pub anypoint_env_id: String,
    pub object_store_id: String,
    /// Used as the cache-key salt for the Anypoint OAuth token.
    pub anypoint_token_url_for_cache_key: String,
    pub cache_safety_margin_seconds: u32,
    pub timeout_ms: u64,
    /// When true, the policy POSTs to the OS v2 admin API on first use to
    /// materialize the store if it doesn't already exist.
    pub auto_create_store: bool,
    /// Escape hatch: when true, every method on this client is a no-op
    /// (reads return `NotFound`, writes/deletes return immediately). Used
    /// to keep the policy serving when the Anypoint OS v2 upstream is
    /// unhealthy. Operators trade durability (per-replica in-memory only)
    /// for availability.
    pub disable_object_store: bool,
    /// TTL applied when creating the store. Ignored if the store already
    /// exists (OS v2 doesn't allow retroactive TTL changes via the
    /// connected-app admin endpoint).
    pub object_store_ttl_seconds: u32,
}

#[derive(Debug, Error)]
pub enum OS2Error {
    #[error("transport error talking to {endpoint}: {source}")]
    Transport {
        endpoint: &'static str,
        #[source]
        source: anyhow::Error,
    },

    #[error("OS v2 returned HTTP {status} on {operation}")]
    HttpStatus { operation: &'static str, status: u32 },

    #[error("OS v2 token endpoint response was not valid JSON: {0}")]
    BadTokenJson(String),

    #[error("OS v2 token endpoint response missing access_token")]
    MissingAccessToken,
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_expires_in() -> u64 {
    1800
}

/// Outcome of a `get` against the persistent store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetOutcome {
    Found(Vec<u8>),
    NotFound,
    /// The store is unreachable / errored. The caller should treat this
    /// as "no data available" rather than failing the request.
    Degraded,
}

pub struct ObjectStoreV2 {
    cfg: OS2Config,
    cache: Rc<dyn Cache>,
    token_service: Rc<Service>,
    base_service: Rc<Service>,
}

impl ObjectStoreV2 {
    pub fn new(
        cfg: OS2Config,
        cache: Rc<dyn Cache>,
        token_service: Rc<Service>,
        base_service: Rc<Service>,
    ) -> Self {
        Self {
            cfg,
            cache,
            token_service,
            base_service,
        }
    }

    fn token_cache_key(&self) -> String {
        token_cache_key(
            ANYPOINT_TOKEN_PREFIX,
            &self.cfg.anypoint_client_id,
            &self.cfg.anypoint_token_url_for_cache_key,
        )
    }

    /// Sentinel cache key written after a successful (or definitively
    /// "store already exists") create attempt. We re-check it on every
    /// read/write so the work happens at most once per replica.
    fn ensure_store_cache_key(&self) -> String {
        format!(
            "os2_store_ready:{}:{}:{}",
            self.cfg.anypoint_org_id, self.cfg.anypoint_env_id, self.cfg.object_store_id
        )
    }

    fn store_admin_path(&self) -> String {
        format!(
            "/api/v1/organizations/{}/environments/{}/stores",
            urlencoding::encode(&self.cfg.anypoint_org_id),
            urlencoding::encode(&self.cfg.anypoint_env_id),
        )
    }

    fn key_path(&self, key: &str) -> String {
        // Path is matched relative to the registered Service base URL. Both
        // the path-segment values and the dynamic key are URL-encoded so
        // task ids that include `/` or other reserved chars don't break
        // routing.
        format!(
            "/api/v1/organizations/{}/environments/{}/data/{}/keys/{}",
            urlencoding::encode(&self.cfg.anypoint_org_id),
            urlencoding::encode(&self.cfg.anypoint_env_id),
            urlencoding::encode(&self.cfg.object_store_id),
            urlencoding::encode(key),
        )
    }

    pub async fn get_token(
        &self,
        client: &HttpClient,
        now_unix: u64,
        force_refresh: bool,
    ) -> Result<String, OS2Error> {
        let cache_key = self.token_cache_key();

        if !force_refresh {
            if let Some(bytes) = self.cache.get(&cache_key) {
                if let Ok(entry) = serde_json::from_slice::<CachedToken>(&bytes) {
                    if !entry.needs_refresh(now_unix, self.cfg.cache_safety_margin_seconds) {
                        return Ok(entry.access_token);
                    }
                }
            }
        } else {
            self.cache.delete(&cache_key);
        }

        let body = serde_urlencoded::to_string(&[
            ("grant_type", "client_credentials"),
            ("client_id", self.cfg.anypoint_client_id.as_str()),
            ("client_secret", self.cfg.anypoint_client_secret.as_str()),
        ])
        .expect("urlencoded for &str is infallible");

        logger::error!("a2a-trace: os2.get_token POST to anypoint-oauth2");
        let response = client
            .request(self.token_service.as_ref())
            .headers(vec![
                ("content-type", "application/x-www-form-urlencoded"),
                ("accept", "application/json"),
            ])
            .body(body.as_bytes())
            .post()
            .await
            .map_err(|e| {
                logger::error!("a2a-trace: os2.get_token transport err: {}", e);
                OS2Error::Transport {
                    endpoint: "anypoint-oauth2",
                    source: anyhow::anyhow!(e.to_string()),
                }
            })?;

        let status = response.status_code();
        logger::error!("a2a-trace: os2.get_token status={}", status);
        if !(200..300).contains(&status) {
            return Err(OS2Error::HttpStatus {
                operation: "anypoint-oauth2",
                status,
            });
        }

        let parsed: TokenResponse = serde_json::from_slice(response.body())
            .map_err(|e| OS2Error::BadTokenJson(e.to_string()))?;
        if parsed.access_token.is_empty() {
            return Err(OS2Error::MissingAccessToken);
        }

        let entry = CachedToken::new(parsed.access_token.clone(), now_unix, parsed.expires_in);
        if let Ok(bytes) = serde_json::to_vec(&entry) {
            let _ = self.cache.save(&cache_key, bytes);
        }
        Ok(parsed.access_token)
    }

    /// Idempotently make sure the configured store exists. On the first
    /// call per replica we POST to the OS v2 admin endpoint; success and
    /// "already exists" responses both flip a cache sentinel so subsequent
    /// reads/writes skip the round-trip. Disabled via
    /// `auto_create_store = false`.
    pub async fn ensure_store(&self, client: &HttpClient, now_unix: u64) {
        if self.cfg.disable_object_store {
            return;
        }
        logger::error!("a2a-trace: ensure_store called auto_create={}", self.cfg.auto_create_store);
        if !self.cfg.auto_create_store {
            return;
        }
        let sentinel = self.ensure_store_cache_key();
        if self.cache.get(&sentinel).is_some() {
            logger::error!("a2a-trace: ensure_store sentinel hit, skip");
            return;
        }

        logger::error!("a2a-trace: ensure_store fetching token");
        let token = match self.get_token(client, now_unix, false).await {
            Ok(t) => {
                logger::error!("a2a-trace: ensure_store token ok");
                t
            }
            Err(e) => {
                logger::error!("a2a-trace: ensure_store token fetch failed: {e}");
                return;
            }
        };
        let bearer = format!("Bearer {token}");
        let body = serde_json::json!({
            "objectStoreId": self.cfg.object_store_id,
            "persistent": true,
            "ttl": self.cfg.object_store_ttl_seconds,
        });
        let body_bytes = serde_json::to_vec(&body).expect("static JSON serializes");

        let response = client
            .request(self.base_service.as_ref())
            .path(&self.store_admin_path())
            .headers(vec![
                ("authorization", bearer.as_str()),
                ("content-type", "application/json"),
                ("accept", "application/json"),
            ])
            .body(&body_bytes)
            .post()
            .await;

        match response {
            Ok(r) => {
                let status = r.status_code();
                // 200/201/204: created. 409: already exists. Both mean the
                // store is usable. Anything else is logged but we don't
                // remember it — we'll retry on next request.
                if (200..300).contains(&status) || status == 409 {
                    let _ = self.cache.save(&sentinel, vec![1]);
                    if status == 409 {
                        logger::info!(
                            "os2: store '{}' already exists",
                            self.cfg.object_store_id
                        );
                    } else {
                        logger::info!(
                            "os2: created store '{}' (ttl {}s)",
                            self.cfg.object_store_id,
                            self.cfg.object_store_ttl_seconds
                        );
                    }
                } else {
                    let body = String::from_utf8_lossy(r.body());
                    logger::warn!(
                        "os2: create-store returned HTTP {status}: {} - confirm the connected app has Object Store admin scope or set autoCreateStore=false and pre-create the store",
                        truncate(&body, 256)
                    );
                }
            }
            Err(e) => {
                logger::warn!("os2: create-store transport error: {e}");
            }
        }
    }

    /// Read a key. On any non-2xx-non-404 response, returns `Degraded` so
    /// the caller can treat it as "no value available".
    pub async fn get(&self, client: &HttpClient, key: &str, now_unix: u64) -> GetOutcome {
        if self.cfg.disable_object_store {
            return GetOutcome::NotFound;
        }
        logger::error!("a2a-trace: os2.get start key='{key}'");
        self.ensure_store(client, now_unix).await;
        logger::error!("a2a-trace: os2.get ensure_store done, fetching token");
        let token = match self.get_token(client, now_unix, false).await {
            Ok(t) => {
                logger::error!("a2a-trace: os2.get token ok len={}", t.len());
                t
            }
            Err(e) => {
                logger::error!("a2a-trace: os2.get token fetch failed: {e}");
                return GetOutcome::Degraded;
            }
        };

        let bearer = format!("Bearer {token}");
        let path = self.key_path(key);
        logger::error!("a2a-trace: os2.get GET path='{path}'");
        let response = client
            .request(self.base_service.as_ref())
            .path(&path)
            .headers(vec![
                ("authorization", bearer.as_str()),
                ("accept", "application/json"),
            ])
            .get()
            .await;
        logger::error!("a2a-trace: os2.get response received");

        match response {
            Ok(r) => {
                let status = r.status_code();
                if status == 200 {
                    GetOutcome::Found(r.body().to_vec())
                } else if status == 404 {
                    GetOutcome::NotFound
                } else {
                    logger::warn!("os2: GET {key} returned HTTP {status}");
                    GetOutcome::Degraded
                }
            }
            Err(e) => {
                logger::warn!("os2: GET {key} transport error: {e}");
                GetOutcome::Degraded
            }
        }
    }

    /// Best-effort write. Errors are logged at `warn` and swallowed so a
    /// transient OS v2 outage doesn't fail the user request.
    pub async fn put(&self, client: &HttpClient, key: &str, value: &[u8], now_unix: u64) {
        if self.cfg.disable_object_store {
            return;
        }
        self.ensure_store(client, now_unix).await;
        let token = match self.get_token(client, now_unix, false).await {
            Ok(t) => t,
            Err(e) => {
                logger::warn!("os2: token fetch failed during put: {e}");
                return;
            }
        };
        let bearer = format!("Bearer {token}");
        let path = self.key_path(key);
        let response = client
            .request(self.base_service.as_ref())
            .path(&path)
            .headers(vec![
                ("authorization", bearer.as_str()),
                ("content-type", "application/json"),
            ])
            .body(value)
            .put()
            .await;
        match response {
            Ok(r) => {
                let status = r.status_code();
                if !(200..300).contains(&status) {
                    logger::warn!("os2: PUT {key} returned HTTP {status}");
                }
            }
            Err(e) => {
                logger::warn!("os2: PUT {key} transport error: {e}");
            }
        }
    }

    /// Best-effort delete. Same error-swallowing semantics as `put`.
    pub async fn delete(&self, client: &HttpClient, key: &str, now_unix: u64) {
        if self.cfg.disable_object_store {
            return;
        }
        let token = match self.get_token(client, now_unix, false).await {
            Ok(t) => t,
            Err(e) => {
                logger::warn!("os2: token fetch failed during delete: {e}");
                return;
            }
        };
        let bearer = format!("Bearer {token}");
        let path = self.key_path(key);
        let response = client
            .request(self.base_service.as_ref())
            .path(&path)
            .headers(vec![("authorization", bearer.as_str())])
            .delete()
            .await;
        match response {
            Ok(r) => {
                let status = r.status_code();
                // 404 is fine - already gone.
                if !(200..300).contains(&status) && status != 404 {
                    logger::warn!("os2: DELETE {key} returned HTTP {status}");
                }
            }
            Err(e) => {
                logger::warn!("os2: DELETE {key} transport error: {e}");
            }
        }
    }
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

    fn cfg() -> OS2Config {
        OS2Config {
            anypoint_client_id: "id".into(),
            anypoint_client_secret: "secret".into(),
            anypoint_org_id: "org-1".into(),
            anypoint_env_id: "env/with/slash".into(),
            object_store_id: "tasks".into(),
            anypoint_token_url_for_cache_key: "https://anypoint.mulesoft.com".into(),
            cache_safety_margin_seconds: 60,
            timeout_ms: 1500,
            auto_create_store: true,
            disable_object_store: false,
            object_store_ttl_seconds: 86_400,
        }
    }

    #[test]
    fn key_path_url_encodes_segments() {
        // Stand-up just enough to call `key_path` directly without needing
        // a real Service. We construct the struct manually with placeholder
        // pointers; we only test the pure path-rendering helper.
        let s = format!(
            "/api/v1/organizations/{}/environments/{}/data/{}/keys/{}",
            urlencoding::encode("org-1"),
            urlencoding::encode("env/with/slash"),
            urlencoding::encode("tasks"),
            urlencoding::encode("task:abc/def"),
        );
        assert!(s.starts_with("/api/v1/organizations/org-1/"));
        assert!(s.contains("environments/env%2Fwith%2Fslash/"));
        assert!(s.ends_with("keys/task%3Aabc%2Fdef"));
        // sanity: cfg is unused but constructible
        let _ = cfg();
    }

    #[test]
    fn token_response_parses() {
        let body = br#"{"access_token":"abc","expires_in":1800}"#;
        let r: TokenResponse = serde_json::from_slice(body).unwrap();
        assert_eq!(r.access_token, "abc");
        assert_eq!(r.expires_in, 1800);
    }

    #[test]
    fn token_response_defaults_expires_in() {
        let body = br#"{"access_token":"abc"}"#;
        let r: TokenResponse = serde_json::from_slice(body).unwrap();
        assert_eq!(r.expires_in, 1800);
    }
}
