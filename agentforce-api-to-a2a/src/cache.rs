//! Shared cache helpers.
//!
//! The Flex Gateway worker-shared cache (see `pdk::cache::Cache`) is used as
//! the hot path for two unrelated workloads:
//!
//!   1. OAuth access tokens for Salesforce and for the Anypoint platform.
//!      Lifecycle is short (minutes), per-replica is fine, OS v2 is not
//!      consulted.
//!   2. The `TaskStore` hot layer that fronts Anypoint Object Store v2 (in
//!      `crate::store::task_store`). Lifecycle is short on purpose; OS v2
//!      is the source of truth.
//!
//! This module owns the cache key derivation for tokens and the shared
//! `CachedToken` shape so the two token cache paths stay symmetric.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Cached access-token entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedToken {
    pub access_token: String,
    /// Absolute UNIX seconds at which the IdP considers the token expired.
    pub expires_at_unix: u64,
}

impl CachedToken {
    pub fn new(access_token: String, now_unix: u64, expires_in: u64) -> Self {
        Self {
            access_token,
            expires_at_unix: now_unix.saturating_add(expires_in),
        }
    }

    /// True when the token should be refreshed, taking the configured safety
    /// margin into account.
    pub fn needs_refresh(&self, now_unix: u64, safety_margin_seconds: u32) -> bool {
        let threshold = self
            .expires_at_unix
            .saturating_sub(safety_margin_seconds as u64);
        now_unix >= threshold
    }

    #[allow(dead_code)]
    pub fn ttl_seconds(&self, now_unix: u64) -> u64 {
        self.expires_at_unix.saturating_sub(now_unix).max(1)
    }
}

/// Cache key prefix used by the Salesforce/Agentforce OAuth client.
pub const AGENTFORCE_TOKEN_PREFIX: &str = "agentforce-tokens";

/// Cache key prefix used by the Anypoint platform OAuth client (for OS v2
/// access).
pub const ANYPOINT_TOKEN_PREFIX: &str = "anypoint-os2";

/// Build a stable cache key from `(prefix, client_id, base_url)`.
///
/// The hash never leaks the client_id into log lines and is constant length,
/// so cache backends with key-length limits remain happy.
pub fn token_cache_key(prefix: &str, client_id: &str, base_url: &str) -> String {
    let mut h = Sha256::new();
    h.update(client_id.as_bytes());
    h.update(b"|");
    h.update(base_url.as_bytes());
    let digest = h.finalize();
    format!("{prefix}:{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_stable() {
        let a = token_cache_key(AGENTFORCE_TOKEN_PREFIX, "id", "https://acme.my");
        let b = token_cache_key(AGENTFORCE_TOKEN_PREFIX, "id", "https://acme.my");
        assert_eq!(a, b);
        assert!(a.starts_with("agentforce-tokens:"));
    }

    #[test]
    fn key_differs_when_inputs_change() {
        let a = token_cache_key(AGENTFORCE_TOKEN_PREFIX, "id1", "https://acme.my");
        let b = token_cache_key(AGENTFORCE_TOKEN_PREFIX, "id2", "https://acme.my");
        let c = token_cache_key(AGENTFORCE_TOKEN_PREFIX, "id1", "https://other.my");
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn key_does_not_leak_client_id() {
        let k = token_cache_key(AGENTFORCE_TOKEN_PREFIX, "secret-client-id", "https://x");
        assert!(!k.contains("secret-client-id"));
    }

    #[test]
    fn agentforce_and_anypoint_keys_diverge() {
        let a = token_cache_key(AGENTFORCE_TOKEN_PREFIX, "id", "https://x");
        let b = token_cache_key(ANYPOINT_TOKEN_PREFIX, "id", "https://x");
        assert_ne!(a, b);
    }

    #[test]
    fn needs_refresh_respects_safety_margin() {
        let token = CachedToken::new("abc".into(), 1_000, 100); // expires_at = 1100
        assert!(!token.needs_refresh(1_000, 60));
        assert!(!token.needs_refresh(1_039, 60));
        assert!(token.needs_refresh(1_040, 60)); // 1100 - 60
        assert!(token.needs_refresh(1_500, 60));
    }

    #[test]
    fn ttl_is_at_least_one() {
        let t = CachedToken::new("a".into(), 1_000, 0);
        assert_eq!(t.ttl_seconds(1_000), 1);
    }
}
