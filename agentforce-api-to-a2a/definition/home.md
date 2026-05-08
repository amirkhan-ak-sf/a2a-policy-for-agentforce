# Agentforce API to A2A

Wraps the Salesforce **Agentforce Agents API** with an
**[A2A 0.3.0](https://a2a-protocol.org/v0.3.0/specification/)** JSON-RPC
server surface. Apply the policy to any API in API Manager and clients can
discover the agent at `/.well-known/agent-card.json` and exchange messages
over A2A while the policy fans out to Agentforce on the back end.

## What it does

For an API governed at `<flex-host>/<apim-resource>`:

| Inbound                                                  | What the policy does                                              |
|----------------------------------------------------------|-------------------------------------------------------------------|
| `GET <base>/.well-known/agent-card.json`                 | returns the configured `AgentCard` (200 / `application/json`)     |
| `POST <base>` with JSON-RPC `message/send`               | `startSession` (if no `taskId`) -> `sendMessage` (sync) -> `Task` |
| `POST <base>` with JSON-RPC `tasks/get`                  | reads the cached `Task`                                           |
| `POST <base>` with JSON-RPC `tasks/cancel`               | calls Agentforce `endSession` and marks the `Task` `canceled`     |
| anything else                                            | passthrough (or 404 when `strictMode = true`)                     |

A2A `Task.id == Task.contextId == Agentforce sessionId` so multi-turn
conversations round-trip cleanly without a side mapping table.

## Authentication to Salesforce

The policy implements the **OAuth 2.0 Client Credentials Flow** for
Salesforce External Connected Apps documented in
[Get Started with the Agentforce API](https://developer.salesforce.com/docs/ai/agentforce/guide/agent-api-get-started.html):

```bash
curl https://{MY_DOMAIN_URL}/services/oauth2/token \
  --header 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode 'grant_type=client_credentials' \
  --data-urlencode 'client_id={CONSUMER_KEY}' \
  --data-urlencode 'client_secret={CONSUMER_SECRET}'
```

Tokens are cached in the Flex Gateway worker-shared cache, refreshed
proactively `cacheSafetyMarginSeconds` before expiry. If Agentforce ever
returns HTTP 401 the policy evicts the cached token, mints a fresh one,
and retries the same upstream call exactly once before surfacing a
JSON-RPC `-32603 InternalError` with `data.reason = agentforce_auth_rejected`.

## Task persistence (Anypoint Object Store v2)

A2A `Task` state is persisted to **Anypoint Object Store v2** so the same
session is reachable from any Flex Gateway replica. A short-lived
`taskHotCacheTtlSeconds` PDK shared cache fronts OS v2 to absorb
follow-on reads. Writes are best-effort - a transient OS v2 outage logs
a warning and lets the RPC succeed; subsequent `tasks/get` against that
task will simply return `TaskNotFoundError -32001`.

The operator must pre-create the Object Store v2 store in Anypoint with
the desired TTL (the policy does not create stores; it only reads/writes
keys).

## Agent card configuration

Pick one of four sources via `agentCardSource`. Whichever you choose,
the policy **always overrides** `protocolVersion`, `url`, and
`preferredTransport` on the served card so it cannot lie about where it
is hosted.

### `structured` (default)

Fill in the form fields:

```yaml
agentCardSource: structured
agentCardName: Acme Agentforce Agent
agentCardDescription: A2A wrapper around the Agentforce Agents API.
agentCardVersion: "1.0.0"
agentCardProviderOrganization: Acme, Inc.
agentCardCapabilitiesStreaming: false
agentCardCapabilitiesPushNotifications: false
agentCardDefaultInputModes: text/plain
agentCardDefaultOutputModes: text/plain
agentCardSkillsJson: |
  [
    {
      "id": "general-conversation",
      "name": "General Conversation",
      "description": "Sync conversation with the agent.",
      "tags": ["agentforce"]
    }
  ]
```

### `inline_json`

Paste a complete card into `agentCardJson`. Useful when you want full
control over every field including the security schemes:

```yaml
agentCardSource: inline_json
agentCardJson: |
  {
    "name": "Acme Agentforce Agent",
    "description": "Wraps Agentforce.",
    "version": "1.0.0",
    "capabilities": { "streaming": false, "pushNotifications": false },
    "defaultInputModes": ["text/plain"],
    "defaultOutputModes": ["text/plain"],
    "skills": [{
      "id": "general-conversation",
      "name": "General Conversation",
      "description": "Sync conversation",
      "tags": ["agentforce"]
    }]
  }
```

### `file`

Same field as `inline_json`. The Anypoint API Manager UI does not have a
file-upload widget so paste the contents of your `agent-card.json`:

```bash
$ cat agent-card.json | pbcopy   # macOS
# Then paste into the agentCardJson textarea in API Manager.
```

The discriminator value `file` exists so operators can declare intent
("this card came from a file we ship with our app").

### `url`

The policy registers the URL as an Envoy upstream cluster and fetches the
card lazily on the first `/.well-known/agent-card.json` request, then
memoizes the result for ~10 minutes. Redeploy the policy (or wait for
the cache to expire) to refresh:

```yaml
agentCardSource: url
agentCardUrl: https://internal-cdn.acme.com/cards/agentforce.json
```

### `agentCardOverrideJson` (any source)

Optional JSON object deep-merged on top of whichever card resolved
above. Use it to override a single field without rewriting the whole
card, e.g. add a security scheme:

```yaml
agentCardOverrideJson: |
  {
    "securitySchemes": {
      "MuleSoftClientId": {
        "type": "apiKey",
        "in": "header",
        "name": "client_id"
      }
    }
  }
```

## Quick start: hit the API

After deploying, your operator-supplied URL exposes the A2A surface:

```bash
# 1. Discover the agent.
curl https://your-flex.example.com/agentforce/.well-known/agent-card.json

# 2. Send a message (no taskId yet -> a fresh Agentforce session is started).
curl https://your-flex.example.com/agentforce/ \
  -H 'content-type: application/json' \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "message/send",
    "params": {
      "message": {
        "kind": "message",
        "messageId": "u-001",
        "role": "user",
        "parts": [{"kind":"text","text":"hello"}]
      }
    }
  }'

# 3. Retrieve the resulting Task by id (== Agentforce sessionId).
curl https://your-flex.example.com/agentforce/ \
  -H 'content-type: application/json' \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tasks/get",
    "params": { "id": "abc-123" }
  }'

# 4. Cancel the conversation (calls Agentforce endSession).
curl https://your-flex.example.com/agentforce/ \
  -H 'content-type: application/json' \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tasks/cancel",
    "params": { "id": "abc-123" }
  }'
```

## Configuration reference

| Field | Required | Default | Description |
|---|---|---|---|
| `myDomainUrl` | yes | — | Salesforce My Domain URL. The OAuth `/services/oauth2/token` endpoint is reached at `<myDomainUrl>/services/oauth2/token`. |
| `agentforceApiUrl` | yes | `https://api.salesforce.com/einstein/ai-agent/v1` | Base URL of the Agentforce Agents API. |
| `consumerKey` | yes | — | External Connected App OAuth `client_id`. |
| `consumerSecret` | yes | — | External Connected App OAuth `client_secret`. Reference via Anypoint Secrets Manager. |
| `agentId` | yes | — | Agentforce agent id used in `POST /agents/{id}/sessions`. |
| `bypassUser` | no | `true` | Use the agent-assigned user (correct for `client_credentials`). |
| `cacheSafetyMarginSeconds` | no | `60` | Refresh tokens this many seconds before `expires_in`. |
| `protocolVersion` | no | `0.3.0` | A2A protocol version. Only `0.3.0` is supported in v1. |
| `a2aRpcPath` | no | `/` | Relative path under the governed API where JSON-RPC is accepted. |
| `publicBaseUrl` | yes | — | Externally reachable base URL, written into `AgentCard.url`. |
| `strictMode` | no | `false` | Return 404 for non-A2A requests instead of forwarding upstream. |
| `objectStoreBaseUrl` | yes | — | Region-scoped Anypoint Object Store v2 host. |
| `anypointTokenUrl` | yes | `https://anypoint.mulesoft.com/accounts/api/v2/oauth2/token` | Anypoint platform OAuth token endpoint. |
| `anypointClientId` | yes | — | Anypoint connected-app client id. |
| `anypointClientSecret` | yes | — | Anypoint connected-app client secret. |
| `anypointOrgId` | yes | — | Anypoint organization UUID. |
| `anypointEnvId` | yes | — | Anypoint environment UUID. |
| `objectStoreId` | yes | — | Pre-created OS v2 store name. |
| `taskHotCacheTtlSeconds` | no | `60` | TTL of the per-replica PDK hot cache in front of OS v2. |
| `taskStoreTimeoutMs` | no | `1500` | Per-call timeout for OS v2 reads/writes. |
| `agentCardSource` | no | `structured` | `inline_json` \| `url` \| `structured` \| `file`. |
| `agentCardJson` | conditional | — | Inline JSON for `inline_json` or `file` modes. |
| `agentCardUrl` | conditional | — | URL fetched at runtime for `url` mode. |
| `agentCard*` (structured fields) | no | — | Form fields used when `agentCardSource = structured`. |
| `agentCardOverrideJson` | no | — | JSON object deep-merged on top of the resolved card. |

## Out of scope for v1

- `message/stream` (Server-Sent Events).
- A2A push notifications.
- A2A `FilePart` / `DataPart` (only `TextPart` is wired in v1).
- gRPC and HTTP+JSON A2A transports.
- `agentCardSignatures` (signing the card with JWS).
