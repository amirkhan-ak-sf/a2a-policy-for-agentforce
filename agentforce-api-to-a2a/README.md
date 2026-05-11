# `agentforce-api-to-a2a` Policy

Flex Gateway custom policy that fronts the Salesforce **Agentforce Agents API** with the
**A2A Protocol 0.3.0** JSON-RPC server surface. See [`definition/home.md`](definition/home.md)
for the operator-facing documentation, and [`ROADMAP.md`](ROADMAP.md) for known gaps and
follow-ups.

This policy was created with the Flex Gateway Policy Development Kit (PDK) 1.8.0. To find
the complete PDK documentation, see
[PDK Overview](https://docs.mulesoft.com/pdk/latest/policies-pdk-overview).


## What it does

Translates A2A JSON-RPC requests into Agentforce Agents API calls:

| A2A method     | Agentforce call                                              |
|----------------|--------------------------------------------------------------|
| `message/send` | `POST /agents/{id}/sessions` then `POST /sessions/{id}/messages` |
| `tasks/get`    | hot-cache + Object Store v2 lookup                           |
| `tasks/cancel` | `DELETE /sessions/{id}` and marks the task `canceled`        |

It also serves the configured `AgentCard` at `<base>/.well-known/agent-card.json`.

The runtime is split into a request-phase and a response-phase filter so outbound HTTPS
calls to Salesforce/Agentforce happen in the response phase. This routes around a
connected-mode Flex Gateway WASM body-state outbound trap; see [`ROADMAP.md`](ROADMAP.md)
for the underlying issue.


## Deploying to your own org

Step-by-step. Skip steps you already have set up.

### Prerequisites

- A Salesforce org with **Agentforce** enabled, including at least one agent and an
  **External Connected App** configured for the **Client Credentials Flow**.
- An **Anypoint Platform** account with API Manager and Runtime Manager access.
- A **Flex Gateway** running and registered in either:
  - **Local mode** — Docker Desktop (≥ 4.x), or
  - **Connected mode** — CloudHub managed gateway, EKS / RTF self-managed gateway, etc.
- macOS or Linux developer machine with:
  - **Rust** toolchain (`rustup` recommended).
  - **Docker Desktop** running.
  - **Anypoint CLI v4** (`anypoint-cli-v4`), authenticated to your org.
- Optional but recommended: an **Anypoint connected app** with the
  *Use Object Store with the connected app* scope, for persistent task storage.

### 1. Clone and bootstrap

```bash
git clone <this-repo>
cd agentforce-api-to-a2a
make setup
```

`make setup` installs `cargo-anypoint` and downloads PDK dependencies.

### 2. Set the asset coordinates to your Anypoint org

Open `Cargo.toml` and replace the `group_id` under `[package.metadata.anypoint]` with your
**Anypoint organization UUID** (find it in Anypoint Platform > Access Management > Business
Groups, or in any URL like `https://anypoint.mulesoft.com/exchange/<group_id>/...`):

```toml
[package.metadata.anypoint]
group_id = "<YOUR-ANYPOINT-ORG-UUID>"
definition_asset_id = "agentforce-api-to-a-2-a"
implementation_asset_id = "agentforce-api-to-a-2-a-wasm"
```

You can change the asset ids if you'd like a different name; Anypoint Exchange asset id
rules require each hyphen-separated segment to be purely alphabetic or purely numeric, so
`a2a` becomes `a-2-a`.

### 3. Set up Salesforce

In Salesforce:

1. **Create or pick an Agentforce agent** and copy its agent id (`0Xx…`) — it appears in
   the agent's URL in Setup. You'll use this as `agentId`.
2. **Create an External Connected App** with the *Client Credentials Flow* enabled
   ([Salesforce docs](https://developer.salesforce.com/docs/ai/agentforce/guide/agent-api-get-started.html)).
   Note the **Consumer Key** (`consumerKey`) and **Consumer Secret** (`consumerSecret`).
3. **Verify locally** with curl that the credentials work:

   ```bash
   curl https://<your-my-domain>.my.salesforce.com/services/oauth2/token \
     -H 'Content-Type: application/x-www-form-urlencoded' \
     --data-urlencode 'grant_type=client_credentials' \
     --data-urlencode 'client_id=<consumerKey>' \
     --data-urlencode 'client_secret=<consumerSecret>'
   ```

   You should get a JSON response with `access_token` and `instance_url`.

### 4. (Optional) Set up Anypoint Object Store v2

Only needed if you want **persistent task storage and full multi-turn conversation
history** in the response. If skipped, set `disableObjectStore: true` in the policy
config.

1. In Anypoint, **create a connected app** with the
   *Use Object Store with the connected app* scope (and *Object Store Admin* if you want
   the policy to auto-create the store on first use).
2. Note the **client id** (`anypointClientId`) and **client secret**
   (`anypointClientSecret`).
3. Note your **organization UUID** (`anypointOrgId`) and **environment UUID**
   (`anypointEnvId`). The org UUID is the same as `group_id` in `Cargo.toml`. The env UUID
   is in Anypoint Platform → Access Management → Environments.
4. Decide on a store name (`objectStoreId`), e.g. `agentforce-a2a-tasks`.

> **Note**: at the time of writing, `tasks/get` and `tasks/cancel` still dispatch
> synchronously in the request phase, which trips the connected-mode WASM body-state
> outbound trap when `disableObjectStore: false`. Track [`ROADMAP.md`](ROADMAP.md) #1 for
> the fix. Until that lands, leave `disableObjectStore: true`.

### 5. Build and unit-test

```bash
make build
cargo test --release
```

You should see all unit and integration tests pass.

### 6. Smoke-test locally with Docker

This step exercises the full policy end-to-end against a real Salesforce + real
Agentforce, on a local Flex Gateway in `--mode=local`.

1. Generate a local-mode `registration.yaml` (you'll need a one-time registration token
   from Anypoint Platform → Runtime Manager → Flex Gateway → Add Gateway → Local mode):

   ```bash
   docker run --rm --platform linux/amd64 \
     --entrypoint /usr/local/bin/flexctl \
     -v "$(pwd)/playground/config:/registration" \
     mulesoft/flex-gateway:latest \
     registration create \
       --mode=local \
       --token=<one-time-token> \
       --organization=<your-anypoint-org-uuid> \
       --output-directory=/registration \
       <your-gateway-name>
   ```

   This drops `playground/config/registration.yaml`. Add it to your `.gitignore` — it
   contains a long-lived credential.

2. Edit `playground/config/api.yaml` and replace the placeholder values under
   `spec.policies[0].config` with your real values from steps 3 and 4. At a minimum:
   `myDomainUrl`, `consumerKey`, `consumerSecret`, `agentId`, `publicBaseUrl`, plus the
   Anypoint OS2 fields (or `disableObjectStore: true`).

3. Stage the freshly built artifacts into the playground:

   ```bash
   cp target/wasm32-wasip1/release/agentforce_api_to_a2a_implementation.yaml \
      playground/config/custom-policies/
   cp target/wasm32-wasip1/release/agentforce_api_to_a2a_definition.yaml \
      playground/config/custom-policies/
   ```

   On `mulesoft/flex-gateway:latest`, the local mode parser rejects `format: password` in
   the definition file. Strip those lines (only locally — leave the source `gcl.yaml`
   alone):

   ```bash
   python3 -c '
   from pathlib import Path
   p = Path("playground/config/custom-policies/agentforce_api_to_a2a_definition.yaml")
   p.write_text(p.read_text().replace("      format: password\n", ""))'
   ```

4. Bring up the gateway:

   ```bash
   docker compose -f playground/docker-compose.yaml up -d
   docker compose -f playground/docker-compose.yaml logs -f local-flex
   ```

5. Test the endpoints:

   ```bash
   curl http://localhost:8081/.well-known/agent-card.json | jq .

   curl -sS -X POST http://localhost:8081/a2a/v1/rpc \
     -H 'content-type: application/json' \
     --data '{
       "jsonrpc":"2.0","id":1,
       "method":"message/send",
       "params":{
         "message":{
           "kind":"message","messageId":"u-001","role":"user",
           "parts":[{"kind":"text","text":"hello"}]
         }
       }
     }' | jq .
   ```

   The first call returns a `Task` with the agent's reply. To continue the conversation,
   pass the returned `result.id` as `taskId` on subsequent `message/send` calls.

   When done:

   ```bash
   docker compose -f playground/docker-compose.yaml down
   ```

### 7. Publish to Anypoint Exchange

Once the local smoke test works, publish a development build:

```bash
make publish
```

This produces:

- `<group_id>:agentforce-api-to-a-2-a-dev:<version>` — the policy definition asset.
- `<group_id>:agentforce-api-to-a-2-a-wasm-dev:<version>` — the policy implementation
  asset (the WASM binary).

The version is derived from `Cargo.toml` plus a build timestamp. To publish a release
asset (no `-dev` suffix, immutable version), use `make release` after bumping the version
in `Cargo.toml`.

### 8. Apply the policy to an API in API Manager

You need an **API instance** (Flex Gateway technology, deployed to your gateway target).
You can create one in the Anypoint UI or via CLI.

#### 8a. Create the API instance (CLI)

```bash
anypoint-cli-v4 api-mgr api manage \
  <some-asset-id-in-exchange> 1.0.0 \
  -f -p \
  --apiInstanceLabel agentforce-a2a \
  --uri https://api.salesforce.com \
  --port 80 \
  --path /agentforce-a2a \
  --scheme http \
  --type http \
  --deploymentType hybrid \
  --environment <your-env-name>
```

Note the **API instance id** that the command prints; you'll use it below.

> The upstream URI (`--uri`) doesn't really matter for this policy — the policy
> short-circuits for the A2A surface and replaces the response in the response phase. Any
> reachable HTTP endpoint that returns 2xx works (`https://api.salesforce.com` is fine and
> avoids needing an extra service).

#### 8b. Deploy the API instance to your Flex Gateway

```bash
anypoint-cli-v4 api-mgr api deploy <api-instance-id> \
  --target <flex-gateway-id> \
  --gatewayVersion 1.13.0 \
  --environment <your-env-name>
```

The gateway id is the UUID printed by
`anypoint-cli-v4 runtime-mgr gateways managed list` (or `gateways self-managed list` for
self-managed gateways like EKS/RTF).

#### 8c. Apply the policy

Save your real values into `policy-config.json`:

```json
{
  "myDomainUrl": "https://<your-my-domain>.my.salesforce.com",
  "agentforceApiUrl": "https://api.salesforce.com",
  "agentforceApiBasePath": "/einstein/ai-agent/v1",
  "consumerKey": "<consumerKey>",
  "consumerSecret": "<consumerSecret>",
  "agentId": "<0Xx-agent-id>",
  "bypassUser": true,
  "cacheSafetyMarginSeconds": 60,

  "protocolVersion": "0.3.0",
  "a2aRpcPath": "/a2a/v1/rpc",
  "publicBaseUrl": "https://<gateway-public-host>/<api-instance-path>",
  "strictMode": true,

  "objectStoreBaseUrl": "https://object-store-us-east-1.anypoint.mulesoft.com",
  "anypointTokenUrl": "https://anypoint.mulesoft.com/accounts/api/v2/oauth2/token",
  "anypointClientId": "<anypoint-cc-client-id>",
  "anypointClientSecret": "<anypoint-cc-client-secret>",
  "anypointOrgId": "<your-anypoint-org-uuid>",
  "anypointEnvId": "<your-anypoint-env-uuid>",
  "objectStoreId": "agentforce-a2a-tasks",
  "autoCreateStore": true,
  "disableObjectStore": true,
  "objectStoreTtlSeconds": 86400,

  "agentCardSource": "structured",
  "agentCardName": "My Agentforce Agent",
  "agentCardDescription": "Agentforce agent fronted by A2A.",
  "agentCardVersion": "1.0.0",
  "agentCardProviderOrganization": "My Org",
  "agentCardProviderUrl": "https://example.com",
  "agentCardCapabilitiesStreaming": false,
  "agentCardCapabilitiesPushNotifications": false,
  "agentCardDefaultInputModes": "text/plain",
  "agentCardDefaultOutputModes": "text/plain",
  "agentCardSkillsJson": "[{\"id\":\"chat\",\"name\":\"Chat\",\"description\":\"General conversation\",\"tags\":[\"agentforce\"]}]"
}
```

Then apply:

```bash
anypoint-cli-v4 api-mgr policy apply <api-instance-id> agentforce-api-to-a-2-a-dev \
  --policyVersion <version-from-make-publish> \
  --groupId <your-anypoint-org-uuid> \
  --configFile ./policy-config.json \
  --environment <your-env-name>
```

`<version-from-make-publish>` looks like `1.0.0-20260508202029`. Use the same value for
`agentforce-api-to-a-2-a-dev` published in step 7.

> **Tip**: keep `disableObjectStore: true` for the first deployment. After verification,
> see [`ROADMAP.md`](ROADMAP.md) before flipping it to `false`.

### 9. Verify the deploy

Use the diagnostic ladder to confirm everything works:

```bash
URL='https://<gateway-public-host>/<api-instance-path>'
RPC="$URL/"

# Agent card (no outbound; quick liveness check)
curl -sS "$URL/.well-known/agent-card.json" | jq '{name, url, protocolVersion, skills: (.skills|length)}'

# Malformed JSON -> 200 -32700
curl -sS -X POST "$RPC" -H 'content-type: application/json' --data 'not-json'

# Unknown method -> 200 -32601
curl -sS -X POST "$RPC" -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"foo/bar","params":{}}'

# tasks/cancel for missing task -> 200 -32001
curl -sS -X POST "$RPC" -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"tasks/cancel","params":{"id":"x"}}'

# message/send -> 200 with Task containing the agent reply
curl -sS -X POST "$RPC" -H 'content-type: application/json' --data '{
  "jsonrpc":"2.0","id":10,
  "method":"message/send",
  "params":{
    "message":{
      "kind":"message","messageId":"u-001","role":"user",
      "parts":[{"kind":"text","text":"hello"}]
    }
  }
}'
```

If any of these returns `404 Not Found` with `content-length: 0` and no `content-type`,
that's the connected-mode WASM trap signature — see [`ROADMAP.md`](ROADMAP.md) #1 / #3.

### 10. Multi-turn

Take the `result.id` from the first `message/send` response and pass it as `taskId` on
the next call. The policy reuses the same Salesforce Agentforce session and skips
`startSession`:

```bash
TID=<result.id from previous response>

curl -sS -X POST "$RPC" -H 'content-type: application/json' --data "{
  \"jsonrpc\":\"2.0\",\"id\":11,
  \"method\":\"message/send\",
  \"params\":{
    \"message\":{
      \"kind\":\"message\",
      \"messageId\":\"u-002\",
      \"role\":\"user\",
      \"taskId\":\"$TID\",
      \"contextId\":\"$TID\",
      \"parts\":[{\"kind\":\"text\",\"text\":\"can you check inventory for MULETEST0\"}]
    }
  }
}"
```


## Make command reference

This project has a Makefile that includes different goals that assist the developer
during the policy development lifecycle. For more information about the Makefile, see
[Makefile](https://docs.mulesoft.com/pdk/latest/policies-pdk-create-project#makefile).

### Setup

`make setup` installs the Policy Development Kit internal dependencies for the rest of
the Makefile goals.

### Build

`make build` compiles the WebAssembly binary of the policy and regenerates the policy
asset files (`definition.yaml`, `implementation.yaml`, `gcl.yaml`).

### Run

`make run` runs the current build of the policy in a containerized Flex Gateway. The
`playground/config/registration.yaml` file generated by a Flex Gateway local-mode
registration must be present. See deploy step 6 above.

### Test

`make test` runs unit tests and integration tests. Equivalent to
`cargo test --release`.

### Publish / Release

`make publish` publishes a development asset (`-dev` suffix, mutable timestamp version).
`make release` publishes the immutable release asset using the version from `Cargo.toml`.
See `definition/home.md` for the published asset documentation.
