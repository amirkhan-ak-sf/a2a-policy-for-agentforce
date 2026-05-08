// Copyright 2026 Salesforce, Inc. All rights reserved.

//! End-to-end happy path for `message/send`.
//!
//! Uses `pdk-unit`'s in-process Proxy-Wasm stub: no Docker, no Flex, no real
//! Envoy. The test wires four mock HTTP backends (Salesforce OAuth,
//! Agentforce API, Anypoint OAuth, Anypoint OS v2), then drives a
//! JSON-RPC `message/send` POST through the policy and asserts the
//! response shape.

use agentforce_api_to_a2a::*; // re-exports nothing yet; relies on the policy entrypoint linkage
use pdk_unit::{Backend, UnitHttpMessage, UnitHttpRequest, UnitHttpResponse, UnitTestBuilder};
use std::cell::RefCell;
use std::rc::Rc;

/// Helper backend that switches on path+method.
struct RouterBackend {
    inner: Box<dyn Fn(UnitHttpRequest) -> UnitHttpResponse>,
}

impl RouterBackend {
    fn new<F: Fn(UnitHttpRequest) -> UnitHttpResponse + 'static>(f: F) -> Self {
        Self { inner: Box::new(f) }
    }
}

impl Backend for RouterBackend {
    fn call(&self, req: UnitHttpRequest) -> UnitHttpResponse {
        (self.inner)(req)
    }
}

fn header(req: &UnitHttpRequest, name: &str) -> Option<String> {
    req.header(name).map(|s| s.to_string())
}

fn json(status: u32, body: &str) -> UnitHttpResponse {
    UnitHttpResponse::new(status)
        .with_header("content-type", "application/json")
        .with_body(body.as_bytes().to_vec())
}

#[test]
fn message_send_happy_path() {
    let policy_config = serde_json::json!({
        "myDomainUrl": "https://acme.my.salesforce.com",
        "agentforceApiUrl": "https://api.salesforce.com/einstein/ai-agent/v1",
        "consumerKey": "TEST_CLIENT",
        "consumerSecret": "TEST_SECRET",
        "agentId": "0XXxx0000000000",
        "publicBaseUrl": "https://gw.example.com/agentforce",
        "objectStoreBaseUrl": "https://object-store-us-east-1.anypoint.mulesoft.com",
        "anypointTokenUrl": "https://anypoint.mulesoft.com/accounts/api/v2/oauth2/token",
        "anypointClientId": "ANY_CLIENT",
        "anypointClientSecret": "ANY_SECRET",
        "anypointOrgId": "00000000-0000-0000-0000-000000000001",
        "anypointEnvId": "00000000-0000-0000-0000-000000000002",
        "objectStoreId": "a2a-tasks",
        "agentCardSource": "structured",
        "agentCardName": "Test Agent",
        "agentCardDescription": "Integration test agent",
        "agentCardSkillsJson": r#"[{"id":"s1","name":"Greet","description":"d","tags":["t"]}]"#
    })
    .to_string();

    // Track call counts so we can assert the upstream hit pattern at the end.
    let calls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let calls_for_sf = calls.clone();
    let salesforce = RouterBackend::new(move |req| {
        let path = header(&req, ":path").unwrap_or_default();
        let method = header(&req, ":method").unwrap_or_default();
        calls_for_sf
            .borrow_mut()
            .push(format!("sf {method} {path}"));
        json(
            200,
            r#"{"access_token":"sf-tok","token_type":"Bearer","expires_in":1799}"#,
        )
    });

    let calls_for_af = calls.clone();
    let agentforce = RouterBackend::new(move |req| {
        let path = header(&req, ":path").unwrap_or_default();
        let method = header(&req, ":method").unwrap_or_default();
        calls_for_af
            .borrow_mut()
            .push(format!("af {method} {path}"));
        if method == "POST" && path.contains("/agents/") && path.ends_with("/sessions") {
            return json(200, r#"{"sessionId":"abc-123"}"#);
        }
        if method == "POST" && path.contains("/sessions/abc-123/messages") {
            return json(
                200,
                r#"{"messages":[{"id":"m1","type":"Inform","message":"Hello!"}]}"#,
            );
        }
        json(404, "{}")
    });

    let calls_for_any = calls.clone();
    let anypoint_token = RouterBackend::new(move |req| {
        let path = header(&req, ":path").unwrap_or_default();
        let method = header(&req, ":method").unwrap_or_default();
        calls_for_any
            .borrow_mut()
            .push(format!("any-tok {method} {path}"));
        json(200, r#"{"access_token":"any-tok","expires_in":1800}"#)
    });

    let calls_for_os = calls.clone();
    let object_store = RouterBackend::new(move |req| {
        let path = header(&req, ":path").unwrap_or_default();
        let method = header(&req, ":method").unwrap_or_default();
        calls_for_os
            .borrow_mut()
            .push(format!("os2 {method} {path}"));
        match method.as_str() {
            "GET" => UnitHttpResponse::new(404),
            "PUT" | "DELETE" => UnitHttpResponse::new(204),
            _ => UnitHttpResponse::new(405),
        }
    });

    let mut tester = UnitTestBuilder::default()
        .with_config(policy_config)
        .with_http_upstream_from_authority("acme.my.salesforce.com", salesforce)
        .with_http_upstream_from_authority(
            "api.salesforce.com",
            agentforce,
        )
        .with_http_upstream_from_authority("anypoint.mulesoft.com", anypoint_token)
        .with_http_upstream_from_authority(
            "object-store-us-east-1.anypoint.mulesoft.com",
            object_store,
        )
        .with_entrypoint(configure);

    // Issue an A2A message/send.
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": {
                "kind": "message",
                "messageId": "u1",
                "role": "user",
                "parts": [{"kind":"text","text":"hi"}]
            }
        }
    })
    .to_string();
    let req = UnitHttpRequest::post()
        .with_path("/agentforce/")
        .with_header("content-type", "application/json")
        .with_body(body.into_bytes());
    let resp = tester.request(req);

    assert_eq!(resp.status_code(), 200);
    let v: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    let task = &v["result"];
    assert_eq!(task["id"], "abc-123");
    assert_eq!(task["contextId"], "abc-123");
    assert_eq!(task["status"]["state"], "completed");
    assert_eq!(task["history"][0]["role"], "user");
    assert_eq!(task["history"][1]["role"], "agent");
    assert_eq!(task["artifacts"][0]["parts"][0]["text"], "Hello!");

    // We should have hit Salesforce OAuth at least once and Agentforce
    // start+send at least once each.
    let log = calls.borrow();
    assert!(log.iter().any(|s| s.starts_with("sf POST")), "{log:?}");
    assert!(
        log.iter()
            .any(|s| s.contains("af POST") && s.contains("/agents/")),
        "{log:?}"
    );
    assert!(
        log.iter()
            .any(|s| s.contains("af POST") && s.contains("/messages")),
        "{log:?}"
    );
}
