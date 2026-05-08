# Roadmap

Open follow-ups, in priority order. Last updated 2026-05-08.

## P0 — Production blockers

### 1. Generalize `on_response` deferral to `tasks/get` and `tasks/cancel`

The current build defers only `message/send` to the response phase. `tasks/get` and `tasks/cancel` still dispatch synchronously inside `on_request`. With `disableObjectStore: false`, those paths make outbound HTTPS to Anypoint Object Store v2 from the request body state, which trips the connected-mode Flex Gateway WASM body-state outbound trap (the same bug we routed around for `message/send`).

**Why it matters**: until this lands, OS2 cannot be enabled. Tasks are not persisted between calls and the policy returns only the current turn in `result.history`.

**What to change**: in `src/lib.rs::handle_rpc_request`, replace the synchronous `tasks/get` / `tasks/cancel` arm with a `Flow::Continue(A2APending::DeferredDispatch { ... })` and have `response_filter` dispatch the same way it does for `MessageSend`. The variant can hold the same fields as `MessageSend`.

### 2. Re-enable Object Store v2 (`disableObjectStore: false`)

After P0 #1, flip the production policy config to `disableObjectStore: false`. This unlocks:
- Persistent A2A `Task` state across replicas.
- Full multi-turn `history` populated in every `message/send` reply.
- `tasks/get` / `tasks/cancel` working against real, persisted tasks.

Verify in this order:
1. The Anypoint connected app used in `anypointClientId` has the **Use Object Store with the connected app** scope (and the **Object Store Admin** scope if `autoCreateStore: true`).
2. The OS v2 host (`objectStoreBaseUrl`) is reachable from the gateway. (We saw `flex-gateway-agent` log `Error authorizing object storage … status: 503/504` in one private space; that needs to clear before flipping the flag.)
3. Run the smoke ladder (see `README.md` "Verifying the deploy") with `tasks/get` against a freshly created task.

### 3. File MuleSoft support ticket for the body-state outbound trap

The connected-mode Flex Gateway WASM filter traps when an outbound HTTPS call is issued **after** `request.into_body_state().await`. We have a clean reproducer (the policy works end-to-end on `mulesoft/flex-gateway:latest` in `--mode=local` and fails identically on CloudHub managed Flex 1.12.5 and self-managed Flex 1.13.0 in connected mode). The on-response workaround unblocks production, but the underlying runtime issue should be reported so future PDK policies aren't forced into the same architectural detour.

Reference correlation IDs from this session:
- Pre-fix CloudHub failure: `ec4574e2-12ef-4570-b8b7-affde52a05fe` and `f4e98859-a36f-430a-a355-2eaac0a9dd77`.
- Diag-A2 confirmation that the same outbound succeeds in `RequestHeadersState`: log timestamp `2026-05-08T20:01:56.094Z` (`a2a-trace: diag-A2 agentforce-probe ok session_id_len=36`), then traps after `into_body_state().await`.

## P1 — A2A protocol completeness

### 4. Salesforce session expiry handling

Agentforce sessions expire after a period of inactivity (org-configurable, typically minutes). When a client sends a `message/send` with a stale `taskId`, the next `sendMessage` returns a 4xx that the policy currently surfaces as `agentforce_http_error`. Better behavior: detect the specific Agentforce expired-session response, mint a new session transparently, and retry `sendMessage` exactly once. Track the new `sessionId` in the OS v2 task store so subsequent calls reach the right session.

### 5. `message/stream` (SSE)

A2A 0.3.0 supports streaming via Server-Sent Events. The Agentforce Agents API has a streaming surface as well. Wiring this up requires a different PDK pattern (response body streaming) and likely the `enable_stop_iteration` PDK feature flag. This is the single largest gap in v1.

### 6. A2A push notifications

A2A clients can subscribe to task events via push. Requires implementing `tasks/pushNotificationConfig/set` and `tasks/pushNotificationConfig/get`, persisting the subscription, and emitting events. Not relevant for the synchronous `message/send` flow but expected for full A2A compliance.

### 7. Non-text parts (`FilePart` / `DataPart`)

Currently `concat_text(&parts)` is the only input parsing, and the policy rejects requests whose parts contain only non-text content with `CONTENT_TYPE_NOT_SUPPORTED -32004`. Wire `FilePart` into Agentforce attachment input and `DataPart` into Agentforce structured input.

### 8. Additional A2A transports

A2A 0.3.0 also defines gRPC and HTTP+JSON transports beyond JSON-RPC over HTTP. Adding gRPC requires substantial PDK work (gRPC service support is in PDK but not used in this policy yet). HTTP+JSON is straightforward.

### 9. `agentCardSignatures` (JWS)

Sign the agent card with a JWS so callers can verify the policy hosting it really controls the configured agent identity. Requires a private key in Anypoint Secrets Manager and a small JWS implementation (or pull in `josekit`/equivalent in WASM).

### 10. `referenceTaskIds`

For cross-task references where a message in one task references the result of another. Schema-only change in `src/a2a/types.rs`; mapping work in `src/a2a/mapping.rs`.

## P2 — Resilience / production quality

### 11. Connected-app permission preflight

At policy load time, do a single `GET` against the OS v2 stores listing endpoint with the configured connected-app credentials. If 401/403, fail policy load with a clear actionable message ("Anypoint client must have **Use Object Store with the connected app** scope; current credentials returned …"). Avoids silent runtime degradation.

### 12. Remove diagnostic flags from production gcl.yaml

`diagnosticPreBodyProbe`, `diagnosticPreBodyAgentforceProbe`, `diagnosticContinueFlow` were used to triangulate the connected-mode trap. They no longer have a use; remove from `definition/gcl.yaml`, `src/generated/config.rs`, `src/config.rs`, and the trace logging that depends on them. Keep the `a2a-trace` lines but downgrade to `logger::debug!` so they don't show as `Error` in operator log views.

### 13. Add per-call timeouts back, configured per environment

We removed explicit `timeout(...)` calls during debugging. Re-add as configurable values (`agentforceApiTimeoutMs`, `os2TimeoutMs`, `oauthTimeoutMs`) once we've verified that connected-mode handles them; the original removal was speculative.

### 14. Better A2A spec-aligned error mapping

A2A 0.3.0 §8 defines error codes. The policy currently uses `-32603 InternalError` for nearly every upstream failure. Map upstream cases more precisely:
- Agentforce 401/403 → `AuthenticatedExtendedCardNotConfiguredError -32007` or similar.
- Salesforce 429 → A2A rate-limit error, with `Retry-After` propagated.
- Network timeouts → `-32603` with `data.reason = transport_timeout`.

### 15. `message/send` idempotency

A2A clients are expected to make `message/send` idempotent within a `messageId`. If the client retries with the same `messageId` while a prior call is still in flight, the policy should dedupe rather than create a second Agentforce session. Requires a short-lived `messageId → task` lookup in the shared cache.

### 16. Rate limiting / circuit breaker against Salesforce

Agentforce per-org limits are real. A circuit breaker that opens after N consecutive 5xx in a window would stop hammering the org and surface a clean `-32011 ContentTypeNotSupportedError`-style A2A error to clients. Use the PDK `pdk_spike_control` or similar for this.

## P3 — Testing

### 17. PDK integration test for the on-response path

`tests/it_handles_message_send.rs` exists for the request-phase path. Add a parallel `it_handles_message_send_via_response_phase.rs` that exercises the new `on_request → on_response` flow with a wiremocked Agentforce + OS2.

### 18. Multi-turn integration test

End-to-end: `message/send` (no `taskId`) → assert returned `taskId`; second `message/send` with that `taskId` → assert reuse of Agentforce session, history with two turns, etc.

### 19. Failure-mode tests

- Agentforce 401 reactive retry path.
- OS2 GET 5xx → degrade to `NotFound` without crashing.
- Salesforce session expired (Agentforce 4xx) → re-mint behavior (after P1 #4 lands).

## P4 — Operability

### 20. Metrics

Per-RPC-method latency histogram, OAuth cache hit rate, OS2 fallback rate, Agentforce upstream error rate. PDK has Prometheus-compatible metrics primitives; wire them up.

### 21. Structured logging

Replace ad-hoc `logger::error!` / `logger::warn!` calls with structured fields (correlation id, method, task id, sequence id) so operators can filter logs cleanly.

### 22. Health / readiness probe

A small `GET <base>/__a2a/health` endpoint that returns 200 when the policy can reach Salesforce and OS2 (or 503 with which one is down). Distinct from Flex Gateway's own health.

### 23. Operator runbook

Capture the most common ops scenarios (token rotation, agent id change, OS2 store recreation, gateway version upgrade) as a runbook in `definition/runbook.md`.

## P5 — Architecture / refactor (optional)

### 24. Split into three policies

Discussed during this session: card serving, Salesforce bearer minting, and A2A wrapper could each be a separate PDK policy chained on the same API. Card and bearer would be reusable across other Salesforce-fronted APIs. The wrapper still needs to be inbound + WASM, but it'd be smaller. Not a blocker for v1; consider when there's a second consumer.

### 25. Consider a Mule 4 reference implementation

For organizations that need full streaming / SSE / push-notifications and don't have a strict "must run as a Flex Gateway policy" requirement, a Mule 4 app would be a more capable runtime. Would coexist with this policy rather than replace it.
