//! Inbound request classification.
//!
//! Three routes are recognized:
//!
//!   * `GET <base>/.well-known/agent-card.json` -> serve the AgentCard.
//!   * `POST <base><a2aRpcPath>`               -> JSON-RPC dispatch.
//!   * Anything else                           -> passthrough or 404 (per
//!                                                `strictMode`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    AgentCard,
    A2aRpc,
    Passthrough,
}

/// Path suffix that signals the well-known agent-card endpoint. The full
/// path is matched as a suffix so the policy works regardless of where
/// the operator mounts the API in API Manager (e.g.
/// `/agentforce/.well-known/agent-card.json`).
pub const AGENT_CARD_SUFFIX: &str = "/.well-known/agent-card.json";

/// Decide what kind of request we have.
///
/// Notes:
///   * `path` may contain a query string; we strip it before matching.
///   * `a2a_rpc_path` is the policy-relative RPC path (e.g. `/`).
///     The router considers the request a JSON-RPC POST when the URL path
///     ends with the configured RPC path, OR when the configured path is
///     just `/` (in which case any POST that isn't the agent-card route
///     is candidate). This matches the operator expectation that the
///     policy can be mounted on any apim resource.
pub fn classify(method: &str, path: &str, a2a_rpc_path: &str) -> Route {
    let bare = path.split_once('?').map(|(p, _)| p).unwrap_or(path);

    if method.eq_ignore_ascii_case("GET") && bare.ends_with(AGENT_CARD_SUFFIX) {
        return Route::AgentCard;
    }

    if method.eq_ignore_ascii_case("POST") && rpc_path_matches(bare, a2a_rpc_path) {
        return Route::A2aRpc;
    }

    Route::Passthrough
}

fn rpc_path_matches(path: &str, rpc_path: &str) -> bool {
    if rpc_path == "/" {
        // Match the API root with or without trailing slash.
        return path.ends_with('/') || path_has_no_extension_after_last_segment(path);
    }
    let trimmed = rpc_path.trim_end_matches('/');
    let candidate1 = trimmed; // exact suffix without trailing slash
    let candidate2 = format!("{trimmed}/");
    path.ends_with(candidate1) || path.ends_with(&candidate2)
}

/// Heuristic for "the path looks like a resource root, not a static
/// asset" - used when `a2aRpcPath = /`.
fn path_has_no_extension_after_last_segment(path: &str) -> bool {
    match path.rsplit('/').next() {
        Some(last) => !last.contains('.'),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_agent_card() {
        let r = classify("GET", "/agentforce/.well-known/agent-card.json", "/");
        assert_eq!(r, Route::AgentCard);
    }

    #[test]
    fn classifies_agent_card_at_root() {
        let r = classify("GET", "/.well-known/agent-card.json", "/");
        assert_eq!(r, Route::AgentCard);
    }

    #[test]
    fn case_insensitive_method() {
        let r = classify("get", "/.well-known/agent-card.json", "/");
        assert_eq!(r, Route::AgentCard);
    }

    #[test]
    fn classifies_post_root_as_rpc() {
        let r = classify("POST", "/agentforce/", "/");
        assert_eq!(r, Route::A2aRpc);
        let r = classify("POST", "/agentforce", "/");
        assert_eq!(r, Route::A2aRpc);
    }

    #[test]
    fn classifies_post_explicit_path() {
        let r = classify("POST", "/agentforce/rpc", "/rpc");
        assert_eq!(r, Route::A2aRpc);
        let r = classify("POST", "/agentforce/rpc/", "/rpc");
        assert_eq!(r, Route::A2aRpc);
    }

    #[test]
    fn does_not_treat_static_asset_as_rpc_when_root_path() {
        let r = classify("POST", "/agentforce/foo.png", "/");
        assert_eq!(r, Route::Passthrough);
    }

    #[test]
    fn passthrough_for_other_methods() {
        let r = classify("PUT", "/agentforce/", "/");
        assert_eq!(r, Route::Passthrough);
    }

    #[test]
    fn passthrough_for_non_matching_paths() {
        let r = classify("POST", "/agentforce/other-thing", "/rpc");
        assert_eq!(r, Route::Passthrough);
    }

    #[test]
    fn strips_query_string() {
        let r = classify("GET", "/.well-known/agent-card.json?foo=bar", "/");
        assert_eq!(r, Route::AgentCard);
    }
}
