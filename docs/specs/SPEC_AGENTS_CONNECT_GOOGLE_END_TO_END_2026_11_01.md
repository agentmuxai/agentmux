# SPEC: an agent connects Google end to end; a person only signs in and consents

**Status:** superseded 2026-11-01 by `SPEC_GOOGLE_PHASE_B_AGENTMUXS_OWN_CLIENT_2026_11_01.md`. The operator dropped bring-your-own clients ("it isnt a real option for users"), so #16317 and #16348 were cancelled. Kept only for the edge-case table (§7) and the `Google` tool design (§5.2), which the Phase B spec carries over.
**Superseded-by:** [`SPEC_GOOGLE_PHASE_B_AGENTMUXS_OWN_CLIENT_2026_11_01.md`](./SPEC_GOOGLE_PHASE_B_AGENTMUXS_OWN_CLIENT_2026_11_01.md)
**Author:** Naki-2@narko.
**Trigger (operator, 2026-11-01), after the first live Google connect:** "each thing you had me do manually needs to have facilities for agents to do themselves. The only thing a human would be needed for is auth, all the other key and form fields should be done by the agent." and, relayed by AgentY: "lets write a spec that goes through the entire path, ensure we implement every part including edge cases, naki-2 can implement it".
**Verified on:** `origin/main` at `5235c6a39` (2026-11-01), AgentMux 0.60.10; the live connect ran on 0.60.10 against project `agentmux-dev`.
**Extends:** `SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md` (the Google spec, below), which it amends in D4 (§6) and D8 (§5.1). **Builds on:** `SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md` (the Vault spec, #6761; its `capture`, `generate` and delivery are reused, nothing in it is duplicated) and `SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` (§5.8).

## 1. Summary

Today, getting an agent from "no Google" to "uses Calendar, Drive and Gmail" took the operator eleven manual steps (§2). Two were authentication. The rest were typing, pasting, choosing and switching. This spec turns every one of the rest into an agent facility, and lists what stays human.

- **Human, always:** signing in to Google (the Cloud console, and the consent flow), Google's 2FA, the "Google hasn't verified this app" **Continue**, and ticking the scopes on the consent screen. Also the OS keychain's own unlock prompt.
- **Agent, with a facility:** getting the client file, putting its secret in the Vault, starting the connect from it, opening the sign-in for the person, following it to the end, binding and granting services to agents, seeing why something is off, reconnecting, and cleaning up the secret afterwards.
- **One new tool, `Google`** (always listed), with actions `status`, `connect`, `connect_status`, `cancel`, `grant`, `ungrant`, `disconnect`. The agent never sees a secret: it hands srv a Vault entry's id.
- **One amendment to the Google spec's D4:** an agent may change grants, as any agent may do anything but speak in the operator's voice (§6). Sends, shares, invites and deletes still wait in the host window.
- **Edge cases** (§7) cover a bad client file, Google's unverified screen, denied or partial consent, a timed-out or failed callback, revoked and 7-day-expired tokens, a different Google account, two agents, a revoke mid-call, container agents, another machine, offline and a locked Vault.

## 2. What happened (the first live connect, 2026-11-01)

| # | Step | Who did it | Agent facility today |
|---|---|---|---|
| 1 | Cloud console: consent screen, test users, three APIs, eight scopes, Desktop client | Naki-2, in a Browser pane; the operator signed in | Works. The click path is proven (§5.7) |
| 2 | Download the client JSON | The operator | `Browser` `capture` with `download` exists (0.60.10), unused here |
| 3 | Open the JSON and read two values | The operator, then Naki-2 wrote a text file | `Terminal` `run` with `capture` (JSON path rules) exists |
| 4 | Find where to paste: the dialog showed only "Connect with OAuth"; **Client type** and the two fields are on the next panel | The operator | none: the start is a window action |
| 5 | Choose **Your own client**, paste id and secret, start | The operator | none |
| 6 | Google's sign-in, "Google hasn't verified this app" **Continue**, scope boxes, callback page | The operator | **Human** (auth and consent) |
| 7 | Find out why no tool appeared | Naki-2, three round trips: `/api/v1/google/self/services` answered `{"account": null, "services": []}` with no reason | none |
| 8 | Give the agent the account and its services (Services section, per agent, per service) | The operator | none: `POST /api/v1/host/google` is host-only (D4) |
| 9 | See the new tool | The agent's next `tools/list`: the MCP refreshes its own list every minute, but tells nobody | none |
| 10 | Delete the JSON and the text file from the Desktop | The operator | A file the agent can reach, it can delete |
| 11 | Record what Google did | Naki-2 | n/a |

## 3. What exists (verified in code, 2026-11-01)

- **The start is a websocket RPC, not an agent route.** `account.oauth.start` (`server/agent_handlers/identity.rs:604`, `COMMAND_ACCOUNT_OAUTH_START`) takes `provider`, `name`, `clientId`, `clientSecret`, `clientMode` (`byo` or `workspace`), `saveClient`, `scopes`, `accountId` (reconnect or more scopes). `poll` and `cancel` are the same kind. The agent tools reach srv over HTTP (`crates/mcp`), so no agent path exists. This is the whole of "window-only".
- **The flow:** PKCE S256 on `127.0.0.1:0` (an OS-chosen port, so "port taken" is only a failed bind, `bind failed: …`, `oauth_client.rs:960`), `state` checked, a 5-minute timeout (`SESSION_TIMEOUT`), statuses Pending, UrlAvailable, CodeEmitted, Success, Failed, Picked, ConfirmUser. The callback page says "You can close this tab and return to AgentMux."
- **The saved client** is in the OS keychain (`oauth-client:google`, `oauth_tokens.rs`), reused by the next connect when none is typed (`identity.rs:627`), and copied into each account's token blob so refresh keeps working.
- **A reconnect must be the same Google user** (`check_same_user`, `oauth_client.rs`): subject, else email. Otherwise: "you signed in as X, but this account is Y. Sign in as Y, or connect X as a new account."
- **Refresh:** within 60 s of expiry; `invalid_grant` sets the account to `needs_reauth` ("Reconnect needed"); `admin_policy_enforced` is "Blocked by your organization".
- **Grants:** `POST /api/v1/host/google` (`google_handlers.rs:297`, op `set` and others) is host-only. `/api/v1/google/self/services` is the agent's own view, read by the MCP at start and every 60 s (`crates/mcp/src/main.rs:169`). An agent is bound to **one** Google account (`access::bound_account` takes the first `google` link). A tool shows only with a binding **and** a grant (D4).
- **No `listChanged`:** `crates/mcp` has no `notifications/tools/list_changed`, so a new tool is invisible to a running agent until its client re-lists.
- **The Vault** has `generate`, `capture` (`Terminal` `run` rules, and `Browser` `capture`), `rotate`, `request` and `list` for agents; its kinds have no OAuth-client kind; no agent action deletes an entry. A value an agent made or captured asks nobody; reading an entry a person made asks them (the Vault spec §4.2).
- **Remote operations** (the multi-machine spec) have no Google or account op.
- **Browser downloads** for agent-opened panes are the remainder of task #196.

## 4. The path, step by step

Who does each step, and the facility. **H** is a person signing in or consenting; everything else is the agent's.

| Step | Who | Facility (new unless marked) |
|---|---|---|
| 0. See where things stand | agent | `Google` `status` (§5.2) |
| 1. Create or reuse the Google project, consent screen, test users, APIs, scopes, Desktop client | agent in a Browser pane; **H** only for the console's sign-in and 2FA | Existing `Browser`; the `google-setup` skill gains the click path of §5.7 |
| 2. Get the client file into the Vault | agent | `Browser` `capture` `download` of the console's **Download JSON** into a Vault entry (existing, #196 permitting); or `Terminal` `run` with `capture` on a file already on disk (existing). Result: one Vault entry holding the JSON, never shown |
| 3. Start the connect from that entry | agent | `Google` `connect {client: <entry id>, mode?, services?, account?}` (§5.2). srv reads the entry, parses it (§5.3), saves the client in the keychain, starts the flow |
| 4. Open Google's sign-in for the person | agent | srv returns the URL with the session; the agent opens it with `Browser` `open` (the person's own profile if they share it) and `handoff` ("Sign in to Google, click Continue on the unverified screen, tick every scope"); or `open_in: "system"` has the host open the system browser (§5.4) |
| 5. Sign in, consent | **H** | The only human step |
| 6. Follow it to the end | agent | `Google` `connect_status {session}` returns state and result (§5.2): granted and missing services, the account's email, the reason on failure |
| 7. Bind and grant services to agents | agent | `Google` `grant {account, agent, service, level}` (binds as well, §5.5) |
| 8. See the tools | agent | `list_changed` from the MCP (§5.6) |
| 9. Clean up the secret | agent | `Vault` `delete {id}` for an entry the agent made or captured (§5.1); a file the agent can reach, it deletes |
| 10. Later: expiry, revoke, another account | agent | `status` says what is wrong; `connect {account}` reconnects (H consents again); `disconnect` and `ungrant` |

## 5. The facilities

### 5.1 The Vault: one kind, no value leaves

- **New kind `google-oauth-client`** (Agent2-6's review, 2026-11-01). Parts `client_json` (the whole file) and `client_secret` (kept as its own hidden part too, so command output and logs that echo only the secret are scrubbed like any Vault value); fields `client_id`, `client_type`, `project`, plus the usual notes and expires. `Browser` `capture` of a download, or `Terminal` `capture` of a file's whole text, can target it. **Validation runs at capture, before anything is stored** (at most 16 KiB, an `installed` Desktop client, the id suffix, a non-empty secret, §5.3), so a wrong file never becomes an entry, and the error never quotes the file.
- **`made_by`:** a new column on the entry row, set by `generate` and `capture` to the verified agent id, empty for the person's own entries (migrated in `storage/vault.rs`, default `""`). It is a stored fact, not read from the Activity log, which is display data and can be pruned. Both `delete` and the no-ask read in `connect` check it.
- **`Vault` `delete {id}`:** only an entry this agent made (`made_by`), and only while the person hasn't edited it (`updated_at == created_at`) or granted it. Anything else is refused with "ask the person to delete it in the Vault pane". No new approval prompt in this work. Recorded in Activity as "deleted by <agent>". This closes step 9; it is Agent2-6's lane, reviewed by them.
- **D8 amended.** The Google spec's D8 kept the BYO client out of the Vault because "a Vault entry is something an agent can be granted". It stays out as a **store**: the keychain remains where the client lives. The Vault is the **transit** for one hop, from the capture to the connect. `connect` deletes the entry only **after the keychain save and a successful token exchange** (state `granted`), through the same delete path (Activity: "deleted after Google connect saved it in the keychain"); on any failure the entry stays, so the agent can retry without downloading again. `keep_entry: true` keeps it. An agent granted the entry through `secrets` could use the secret in a command; Google treats a Desktop client's secret as not confidential, and the grant is the Vault's usual per-use approval.
- **Approval:** `connect` reading an entry this agent made (`made_by` is its id) asks nobody, and is recorded in the entry's Activity as a use ("used by Google connect", actor the agent), like every other use. Reading an entry a person made asks them in the Vault's own window (an existing path); that is consent over their secret, not a setup chore.

### 5.2 The `Google` tool

Always listed (it is how an agent gets the others). Each action is an agent-token route, `POST /api/v1/google/setup`, with the agent's verified identity (the `calling_agent()` of the existing Google routes), and the same wording rules as the other Google tools. Results carry no secret and no token.

- **`status`** `{account?}`: every Google account on this machine, each with email, client mode (`builtin`, `byo`, `workspace`), state (`connected`, `reconnect_needed`, `blocked_by_org`, `partly_granted` with what is missing), when it was connected, services on, and the asking agent's own binding and grants. **Never empty without a reason:** with no account it says "no Google account is connected on this machine; `connect` makes one"; with an account but no grant, "account X is connected but not granted to you; `grant` does it".
- **`connect`** `{client?, mode?, services?, account?, name?, open_in?, restart?}`: starts a flow.
  - `client`: a Vault entry id (§5.1). Omitted: the saved keychain client. Neither: the answer says "no Google client is saved; get the client file into the Vault (§4 step 2)", naming the console page.
  - `services`: `calendar`, `drive`, `gmail`, with a level each; they become the scopes asked. Default: the services the person is likely to want are not guessed: sign-in only, then `grant`/`connect {account, services}` adds more (incremental authorization, built).
  - `account`: an existing account's id or email, to reconnect or add services.
  - **Returns** `{session, url, expires_in, account?}` at once. One connect per provider per machine is pending: a second `connect` returns the pending session unchanged, and `restart: true` cancels it first.
- **`connect_status`** `{session}`: `waiting_for_sign_in`, `granted` (with the account, the services granted and any missing), `denied`, `timed_out`, `failed` (a plain sentence, §7), `cancelled`.
- **`cancel`** `{session}`.
- **`grant`** `{account, agent, service, level}` and **`ungrant`**: §5.5. `level` is `read`, `read_write` or `off`.
- **`disconnect`** `{account}`: Google's revoke first, then the keychain entry, the grants and the log (existing behaviour; the build checks that grants and the log go with it, since `deleteidentityaccount` keeps the row when the revoke fails and `require_revoke` is set).

### 5.3 Reading the client file (one parser, in srv)

The window's parser (`parseGoogleClientJson`, built in #6092) and srv's become one set of rules with one shared fixture file (`valid`, `web`, `empty secret`, `not JSON`, `too big`, `wrong id`), tested on both sides. The rules: a JSON object with `installed` (a Desktop client); `client_id` ends `.apps.googleusercontent.com`; non-empty `client_secret`; at most 16 KiB. A `web` file says it is a Web client and what to do. Errors name the field and never contain the secret.

### 5.4 Opening Google's sign-in

srv already knows the authorization URL (`UrlAvailable`). The agent path returns it. Two ways to put it in front of the person, chosen by `open_in`:
- **`browser` (default for a host agent):** the agent opens it in a Browser pane and hands off. Google's sign-in is usually already signed in there (the person's own profile), so the person often only clicks **Continue** and the scope boxes. The loopback callback is served by srv, so a Browser pane, the system browser and a container agent all end in the same place.
- **`system`:** srv asks the host to open the system browser, with a notice in the window naming the agent and the account. The only option when no Browser profile may be shared (a container agent's pane is Incognito by design).
The handoff's reason names the exact screens ("Google hasn't verified this app: click **Continue**, then tick every scope").

### 5.5 Binding and granting

`grant` binds the agent to the account (a `google` identity link) and sets the service's level in one call. It replaces the host-only `POST /api/v1/host/google` set as the way an agent does it; the window's Services section keeps working and calls the same code. Both write a row in the account's log naming who did it (the agent, or the window) and when. Rules:
- An agent has one Google account today (`bound_account`). Granting a second account to an agent that has one **refuses with the sentence "X already has Y; `ungrant` it first, or pass `replace: true`"**; two accounts per agent is a separate piece of work (§11 Q2).
- A level Google didn't grant is refused with what is missing: "Gmail read needs `gmail.readonly`, which wasn't ticked: `connect {account, services: [gmail]}` asks again".
- Granting Gmail on the built-in client is refused as today (`restricted_scope_needs_own_client`).
- A non-blocking notice shows in the host window ("Naki-2 gave Agent5 Calendar read"). It asks nothing.

### 5.6 The new tool appears without a relaunch

`crates/mcp` declares `tools: {listChanged: true}` and sends `notifications/tools/list_changed` when its granted-services list (refreshed every 60 s today) differs, and srv wakes the MCP at once when a grant changes (an SSE or a short poll after `grant`). Whether each client honours it is **checked** in the build with Claude Code, Codex and Gemini. For a client that doesn't, `grant`'s answer says "relaunch to see Calendar", and `Google` `call {tool, action, args}` is the fallback: it forwards one call to `Calendar`, `Drive` or `Gmail` through the same route and the same grants, so the work doesn't wait for a relaunch.

### 5.7 The Cloud console is the agent's

The first connect proved the console path in a Browser pane with the operator only signing in. The `google-setup` skill gets the verified click path: project `agentmux-dev` (or the person's own), Google Auth Platform: Branding (name, support email, authorized domain, developer contact), Audience (External, **Testing**, test users), Data access (the scopes), Clients (Desktop app, then **Download JSON**), and the three APIs. Also:
- **Publishing a BYO project "In production"** (unverified) removes the 7-day token expiry (§7); the agent can do it in Audience when the person wants a long-lived connection.
- **Adding the signed-in account as a test user** fixes Google's "Access blocked" for an account not on the list.
- Google has no public API for creating Desktop clients or the External test-user list (not checked this session); the console is the way, and the form fields in it are the agent's.

### 5.8 Another machine

Tokens never leave the machine that holds the keychain. With the multi-machine spec's `host` argument:
- `Google` `status` is `Op::read_personal`; `grant` and `ungrant` are `Op::write` (the same certificate and trusted-install rules); the remote name is `google.status`, `google.grant`.
- `connect` on another machine is **refused with a sentence** ("Google's sign-in has to happen in a browser on that machine"): the consent step is human and local. The agent there, or a person, runs it. A later step may let the requesting machine's agent start it there and hand the URL back.

## 6. Decision: grants

**Decided by the operator, 2026-11-01, in conversation with Naki-2:** "yes, agents can grant". The recommended answer below stands; the alternative is kept for the record.

**D4 (Google spec) said only the host changes grants**, because "agents hold srv's instance key, so a grant settable over that key could be granted by an agent to itself." The operator's rule is that agents do anything except forge the operator's voice, and the guard on grants was "operator only".

**Recommended:** an agent may `grant` and `ungrant` any agent, itself included, at any level the consent allows. It is recorded as that agent, shown in the account's log and card, and noticed in the host window. The protection that remains is the one that matters most: **a send, share, invite or delete still waits in the host window, which no agent can answer** (Google spec §6), plus the rate limits and untrusted-content marking (§9 there). What this accepts: an agent fooled by a crafted email could turn on more read access for itself. It could already read whatever it was granted, and nothing leaves without a person's yes.

**Alternative:** keep grants human, and add only `request_grant` (the agent asks, the host window answers). Safer; it puts a human click back in every setup.

## 7. Edge cases

Each row: what happens, what the agent does, what a person does, and the test that pins it. "Built" marks behaviour that exists today; the rest is this spec's build.

| Situation | Behaviour | Agent | Person | Test |
|---|---|---|---|---|
| **Client file is wrong** (not JSON, a Web client, no secret, wrong id, over 16 KiB, an entry that isn't a client) | Refused at capture and at connect with a plain sentence naming the problem and the console fix; the secret is never in a message, log or note | Gets the sentence, downloads the right file | none | Shared fixtures, both parsers; log scan finds no secret |
| **Right file, wrong client** (secret reset in the console later; client deleted) | Token exchange or refresh returns `invalid_client`: "Google doesn't know this client any more: download its JSON again" | Re-captures, `connect {client}` | none | Mock token endpoint |
| **Web client used** | Google answers `redirect_uri_mismatch`; the sentence says to create a Desktop app client | Creates one (§5.7) | none | Mock |
| **Calendar, Gmail or Drive API not enabled** in the project | First call answers `accessNotConfigured`: the sentence names the API and the console page | Enables it in the console | sign-in if needed | Mock |
| **Unverified-app screen** ("Google hasn't verified this app") | Expected for every test user in Testing mode, and for a BYO app In production | The handoff reason says to click **Continue** | **Continue** | Live, recorded in the Google spec §8 |
| **Account not on the test list** ("Access blocked", `access_denied` with Google's description) | Mapped to "add X as a test user: Audience page" | Adds the test user in the console | none | Mock callback |
| **Person denies** (`access_denied`) | `connect_status` → `denied`: "you didn't allow AgentMux"; nothing saved; the listener is gone | May `connect` again on request | decides | Mock callback |
| **Partial consent** (a scope unticked) | Built: granted scopes read back. `granted` lists what was and wasn't: "Gmail read is missing `gmail.readonly`"; tools limited to what was granted | `connect {account, services}` asks again | ticks it | Built tests plus a status case |
| **Callback never comes** (the person walked away) | 5 minutes, `timed_out`, the listener is closed, no account made | Offers to start again | none | Timer test with a short timeout |
| **The loopback bind fails** | `failed`: "AgentMux couldn't open its local sign-in port (…)": built (`bind failed`), now with the plain sentence | Retries once, then tells the person | none | Fail the bind in a test |
| **A second connect while one waits** | The pending session is returned, not a second listener; `restart: true` cancels first | Reads the session | none | Two calls, one listener |
| **Connect to an account that's mid-flow in the window** | The window's flow and an agent's use the same one-pending rule | Gets the pending session | none | Same |
| **Refresh token revoked, expired, or the 7-day Testing expiry** | Built: `invalid_grant` → "Reconnect needed". New: `status` says "reconnect needed since <time>; this is probably the 7-day limit of a project in Testing. Publish the project (In production, unverified) for long-lived tokens, or reconnect weekly". A tool call says the same in one sentence. A network error is never read as `invalid_grant` | `connect {account}` | consent | Mock `invalid_grant` vs timeout |
| **Reconnect as a different Google account** | Built: refused ("you signed in as X, but this account is Y …"); nothing changes | `connect` with no `account` makes a new one | signs in as the right one | Built test |
| **Two accounts for one agent** | `grant` refuses a second without `replace` (§5.5) | Chooses | none | Grant test |
| **Two agents granted** | One account, a binding and a level each; rate limits and the send count are per agent and account (built); token refresh is one at a time per account (the build confirms, adds a lock if not) | Each its own | none | Two parallel calls, one refresh |
| **Revoke while a call is in flight** | The grant is checked at the start **and again before the result is returned**: a call answered after an `ungrant` returns "access was removed" and drops the content. A `send_draft` waiting in the window re-checks the grant after the yes; a mail already sent can't be recalled (and the window said so) | Gets the sentence | none | Delay the Google mock, `ungrant`, assert nothing returned |
| **Disconnect while a call is in flight** | The same: "this account was disconnected" | Same | none | Same |
| **Container agent** | Routes are reachable with its token. The sign-in opens in the **system browser** on the host (a container pane is Incognito); the person's callback reaches srv on the host. A file on the person's Desktop is not reachable: the client comes by `Browser` `capture` or the container's own `Terminal` `capture`, into the host's Vault | Same actions | consent | Container fixture |
| **Instance on another machine** | `status` and `grant` work with `host`; `connect` is refused with its sentence (§5.8) | Asks the agent there | consent there | Remote-ops test |
| **Offline at start** | The exchange fails: "can't reach Google"; the session stays `waiting` until its timeout, and retries once the network is back; no account is made until the exchange works | `connect_status` | none | Mock down, then up |
| **Offline later** | A tool call says "Google isn't reachable"; the account state does not change | Retries later | none | Mock |
| **Vault locked** (the OS keychain wants its confirmation) | `connect` reads nothing and says "the Vault is locked: confirm the system prompt, then retry"; never a plain-file fallback | Retries after the person says so | unlocks | Locked-store fake |
| **Keychain can't store the client** | `connect` fails **before** opening the sign-in: "AgentMux can't save the client in the system keychain" | Reports | fixes the keychain | Fake store failing |
| **The Vault entry is gone or isn't a client** | "no such entry" / "this entry isn't a Google client file" | Re-captures | none | Unit |
| **Workspace admin blocks the app** | Built: "Blocked by your organization" with the admin's next step | Reports | the admin acts | Built |
| **Gmail on the built-in client** | Built: refused with the BYO sentence | Uses BYO | none | Built |
| **Two sign-ins at once** (two windows) | One pending session per provider (above) | n/a | n/a | n/a |
| **srv restarts mid-flow** | The pending session is lost; `connect_status` says `unknown_session`; no half account (the account row is written on success) | Starts again | consent again | Restart test |
| **The person deletes the account in the window** | Same as disconnect | `status` says it's gone | none | Same |

## 8. Security

1. **No secret reaches an agent.** The client lives in the Vault, then the keychain; srv reads it. `status`, `connect_status` and every error omit it. A log scan is a test (§9).
2. **`google-oauth-client` entries** are scrubbed in command output like any Vault value (the Vault spec §4.2).
3. **A grant by an agent is a normal write** with the agent recorded as the actor. It can't be made to look like the person's (the one thing no agent may do). The approval window for sends and deletes stays host-only.
4. **The sign-in URL** carries a one-time `state` and PKCE challenge only; the verifier stays in srv. Returning the URL to an agent leaks nothing it can reuse.
5. **A prompt-injected agent** can ask for a connect, which does nothing without a person signing in and consenting. That consent screen is the guard for new access; the host window is the guard for irreversible actions.
6. **Tokens stay in srv** (D2 stands). No new route returns one.

## 9. Tests

- **Unit (srv):** the parser fixtures; `connect` against a mock authorization server and token endpoint (success, `denied`, partial, timeout, `invalid_client`, `invalid_grant` vs network error, `redirect_uri_mismatch`, `accessNotConfigured`); one-pending-per-provider; `status` text for every empty case; grant rules (bind, level, replace, missing scope, built-in Gmail); grant re-check before the result returns; Vault `delete` (own entries only, refused after an edit or a grant); `made_by` set by generate and capture; the secret part scrubbed from command output; the entry kept on a failed exchange; locked Vault; failing keychain.
- **Log scan:** a test that connects with a known secret and greps every log line, status and error for it.
- **MCP:** `Google` always listed; `Calendar`, `Drive` and `Gmail` listed on a grant; `tools/list_changed` sent once per change.
- **Frontend:** the window's Services section and Connect panel call the same code; the shared parser fixtures.
- **Live, on `agentmux-dev`, with a person for the sign-in only:** the whole §4 path by an agent: download, capture, connect, handoff, consent, grant, tools appear, Calendar read, Drive pick, Gmail read, draft, send to self with the host window, disconnect with the revoke checked at myaccount.google.com/permissions, then reconnect. A container-agent pass and an `ungrant`-during-call pass.

## 10. Build plan

One PR per row, in this order, each with its tests and a changeset; the first live pass runs after row 4.

| PR | Contents | Review |
|---|---|---|
| 1 | One parser and fixtures; the `google-oauth-client` kind with its `client_secret` part; the `made_by` column; `Vault` `delete` for own entries; `capture` into the kind | Agent2-6 reviewed the design (2026-11-01, folded into §5.1) and reviews this PR; their stack (#6084, B2 part 2, B3) touches `vault/uses.rs` and `vault/signins.rs`, not `kinds.rs` or the entry table |
| 2 | `Google` `status`, `connect`, `connect_status`, `cancel`, with the one-pending rule and every §7 sentence | muxreview |
| 3 | `grant`, `ungrant`, `disconnect`; the window's Services calls the same code; the in-flight re-check | muxreview |
| 4 | `listChanged` and the `Google` `call` fallback | muxreview |
| 5 | Remote `google.status` and `google.grant` | muxreview |
| 6 | The `google-setup` skill's click path; the Google spec's §3, §7, §8 updated; the corporate record | with 5 |

## 11. Open questions

1. **Grants (§6):** the recommended answer, or `request_grant` only.
2. **Two Google accounts for one agent:** one account per agent stays for now. Proposal: a later piece of work, because the tools' optional `account` argument already assumes it.
3. **Who deletes the Vault entry:** settled with Agent2-6 (§5.1): `connect` deletes it after a successful exchange, `keep_entry` keeps it.
4. **Phase B (AgentMux's own client):** a built-in client id and a secret field in `oauth_client.rs`, and the verification submission, are the operator's and are not part of this spec; they reuse §5 unchanged.
