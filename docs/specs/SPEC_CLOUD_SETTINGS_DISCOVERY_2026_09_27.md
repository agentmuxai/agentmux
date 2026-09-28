# SPEC: AgentMux Cloud settings discovery, and recovering from a dead cloud sign-in

**Date:** 2026-09-27
**Status:** active — slice 1 (§3.1–3.2, discovery used for login and the WebSocket URL) merged in PR #3954; slice 2 (§3.3, §3.5, "Sign in again") in PR #3981; slice 3 (§3.4, per-agent credentials re-provisioned) in PR #3982; slice 4 (`muxbus.cloudconfig`, `isConfigured()` from it) in this PR.
**Author:** Maricon

---

## 1. Problem

The desktop's AgentMux Cloud (MuxBus) settings are compiled in:

| Setting | Where | Consumed by |
|---|---|---|
| Cognito hosted-UI domain | `frontend/.env.production` `VITE_MUXBUS_COGNITO_DOMAIN`; legacy fallback in `AgentMuxConnectPanel.tsx:32-36` | the frontend sends it in `MuxBusLoginReq`; srv's PKCE flow (`muxbus/pkce.rs`) builds the authorize URL and exchanges the code with it |
| Cognito desktop client id | `VITE_MUXBUS_CLIENT_ID` (a production build fails without it, `vite.config.ts:68-96`) | the same |
| Relay REST URL | `cloud_subscriber.rs:48` `MUXBUS_REST_URL`, env override `AGENTMUX_MUXBUS_REST_URL` (`relay.rs:49-54`) | login relay, agent provisioning, WAN keys, leases, inject |
| Relay WebSocket URL | `cloud_subscriber.rs:47` `MUXBUS_WS_URL` (no override) | the cloud subscriber |

After sign-in, srv stores the domain and client id with the tokens (`MuxBusCredentials`, `storage/muxbus.rs:146-155`) and refreshes against them (`pkce.rs:362-414`).

**Consequences:**

1. **Moving the cloud's identity service needs a new desktop build.** A Cognito user pool can't be renamed or moved, so a new pool means a new client id, and every installed build keeps the old one.
2. **A dead sign-in is silent.** When a refresh is rejected permanently, the broker moves the credential to `NeedsReauth` (`broker/state.rs:59,261-266`). Nothing reads that state:
   - the UI only knows about expiry (`HostPopover.tsx:355`, `AgentMuxConnectPanel.tsx:191,285`);
   - the subscriber keeps reconnecting with backoff (`cloud_subscriber.rs:412-437`);
   - the credential is never cleared.

   The user sees "connected" or "expired" at best, never "sign in again".
3. **Dead per-agent credentials are retried forever.** A rejected client-credentials fetch (`invalid_client`) or provisioning failure starts a 60 s cooldown and falls back to the shared token (`agent_credentials.rs:96-124`). The stored `client_id`/`client_secret` is never deleted, so it retries every 60 s indefinitely.

## 2. Goals

1. The desktop learns the cloud's sign-in and relay settings from the cloud at runtime, falling back to today's compiled values.
2. A sign-in that can't work anymore shows up as **"Sign in again"**, and srv stops spinning on it.
3. Per-agent credentials that the cloud no longer recognizes are dropped and re-provisioned.

**Non-goals:**
- Choosing between clouds (dev/staging/prod) in the UI. The relay REST override (`AGENTMUX_MUXBUS_REST_URL`) already selects one, and discovery follows from it.
- Changing the relay protocol.

## 3. Design

### 3.1 The discovery document

`GET {relay REST base}/.well-known/agentmux-cloud.json`: public, no auth, cached by the relay for 5 minutes.

```json
{
  "version": 1,
  "api": "https://muxbus.agentmux.ai",
  "ws": "wss://muxbus-ws.agentmux.ai",
  "console": "https://cloud.agentmux.ai",
  "cognito": { "domain": "https://auth.muxbus.agentmux.ai", "clientId": "…", "region": "us-east-1", "userPoolId": "…" }
}
```

- Everything in it is already public: the domain and the PKCE client id are in every sign-in URL, and the pool id is in every token's `iss`.
- Unknown fields are ignored.
- A `version` other than 1 is treated as "no document".

### 3.2 srv: resolving settings (`muxbus/discovery.rs`)

- `cloud_settings()` fetches the document from `relay::rest_base_url()`. Details:
  - a 5 s timeout;
  - the result is cached in memory for 5 minutes, and the last good copy is kept;
  - on any failure, the compiled defaults are used, with no error surfaced for that alone.
  - a failed fetch keeps the last good copy, and also waits the 5 minutes before asking again.
- **Login:** `muxbus.login` uses the discovered `cognito.domain` and `cognito.clientId` when present. It falls back to the values in `MuxBusLoginReq` (the compiled ones), then to nothing (a "cloud not configured" error). The frontend keeps sending its compiled values as the fallback.
- **WebSocket:** the cloud subscriber connects to the discovered `ws`, falling back to `MUXBUS_WS_URL`.
- **REST:** unchanged. `rest_base_url()` stays the root of trust, because the document is fetched from it.
- **`muxbus.cloudconfig`** (new RPC, slice 4): returns the resolved settings and their source (`discovered` or `default`). Only needed for a build with **no** compiled client id. Production builds always have one (`vite.config.ts` refuses to build without it), so slice 1 doesn't need it.
  - Shape (`MuxBusCloudConfigResp`, camelCase): `source`, `api`, `ws`, `console`, `cognitoDomain`, `clientId`, `region`, `userPoolId`.
  - `discovered`: everything but `api` from the document; the last good copy counts, even while later fetches fail.
  - `default` (no document, or discovery failed with no earlier copy): `ws` is `MUXBUS_WS_URL`, and the console and all Cognito fields are empty. srv has no compiled sign-in settings; only the frontend build does.
  - `api` is always `rest_base_url()`, the REST base srv actually uses, whatever the document says.
  - It reads no credential and touches no keychain.

### 3.3 srv: a sign-in that can't work anymore

A stored credential is **stale** when:
- the broker has it in `NeedsReauth` because a refresh was refused (a permanent refresh failure: 4xx other than 408/429), and no fresh token has been stored since. `NeedsReauth` reached by piling up transient failures (e.g. offline) is not stale; or
- the discovered `cognito.clientId` differs from the stored `client_id`. It was issued by a user pool this cloud no longer uses.

When stale:
- `muxbus.status` reports `needs_reauth: true` along with the stored email;
- the subscriber stops reconnecting, logs the reason once, and waits for a new sign-in. A sign-in here wakes it at once; one made by another channel sharing the store is noticed by a once-a-minute local recheck;
- no refresh is attempted against a pool the cloud has left.

The stored credential is **not deleted automatically**. The user's next sign-in replaces it, as `muxbus_save` already does, and an explicit disconnect clears it.

### 3.4 srv: per-agent credentials the cloud no longer recognizes

- When the client-credentials token request for an agent answers `invalid_client` (HTTP 400/401), or provisioning answers 401/403 for that agent's client, srv **deletes that agent's stored credential row**. The next use re-provisions it through `/agents/provision`.
- The existing 60 s cooldown still limits how often that happens.
- Other failures (network, 5xx, 429) keep today's behaviour.

### 3.5 Frontend

- `useMuxBusStatus` reads `needs_reauth`. The cloud row in the host popover and the AgentMux Cloud panel show **"Sign in again"**, with the stored email, and the same connect action as today.
- `isConfigured()` is true when the build has a client id **or** `muxbus.cloudconfig` returns one (slice 4). Details (`muxbus-cloud-config.ts`):
  - a build with a compiled client id never calls the RPC, and signs in with its compiled pair as before;
  - a build without one asks from `refresh()`, without awaiting it, so the status never waits on it. One request is shared by every controller, and a client id is kept for the session once known. Until then each `refresh()` asks again; srv answers from its 5-minute cache;
  - `connect()` signs in with the discovered pair in that case. srv still prefers its own copy (§3.2).

## 4. Slices

| Slice | Content |
|---|---|
| **1** | §3.1–3.2: discovery in srv, used for login and the WebSocket URL |
| **2** | §3.3 and §3.5: `needs_reauth`, the subscriber stops spinning, "Sign in again" in the UI |
| **3** | §3.4: per-agent credentials are dropped and re-provisioned |
| **4** | `muxbus.cloudconfig`, and `isConfigured()` from it (§3.5), for builds without a compiled client id |

## 5. Testing

- **Unit:**
  - document parsing: version, missing fields, unknown fields;
  - the cache and fallback order;
  - stale detection, both from `NeedsReauth` and from a client-id mismatch;
  - the `invalid_client` classification.
- **Against a real cloud:**
  - sign in with a build that has no compiled client id; the settings come from discovery;
  - point the relay at a new user pool, and a signed-in desktop shows "Sign in again" without a restart;
  - agents re-provision on their next start.
