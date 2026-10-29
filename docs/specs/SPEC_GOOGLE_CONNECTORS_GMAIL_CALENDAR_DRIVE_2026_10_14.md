# SPEC: Google in Connectors: Gmail, Calendar and Drive for every agent

**Status:** active — G0's srv half is built in #5310 (G0a, 2026-10-21): the Google authorization parameters, incremental scopes on an existing account, granted-scope read-back, the email from the ID token, refresh in srv (`needs_reauth` on `invalid_grant`), revoke on delete, the Google validator, token-blob parsing (§2.3's GitHub bug) and the BYO client saved in the keychain (`identity/oauth_client.rs`, `identity/oauth_tokens.rs`). G0's frontend half is built too (G0b, 2026-10-21): the Connect panel uses the saved client or shows the BYO fields once with the console steps (`OAuthClientSection.tsx`), and the card shows the email, its client tag, `Connected` / `Reconnect needed` / `Blocked by your organization`, a partial grant, Reconnect, and Disconnect's retry (`account-card.tsx`, `OAuthAccountSection.tsx`). G1a, Calendar reads, is built (2026-10-24, §8). G2's reads are built (2026-10-28): the `Drive` tool's `search_files`, `get_file_metadata` and `read_file_content` on `drive.file`, `pick_files`, and "Give agents files from Drive…"; how Google's picker behaves is still to be confirmed against a real account (§3.3, §11 Q4). G3 is built except attachment downloads (2026-10-29): the `Gmail` tool's `search_threads`, `get_thread` and `list_labels` on `gmail.readonly`, and `create_draft`, `update_draft`, `label_thread`, `send_draft` and `trash_thread`, offered only on a client the person brought. Registering AgentMux's own client, G1b, the Drive writes, Gmail's drafts and send, and G4 follow. **Refreshed 2026-10-24** (Naki-2, at the operator's request, "lets get naki back on google cloud, the work along the gmail/drive/calendar connectors"): the tools are three action-style tools (D3, §5), the Vault's place is settled (D2, D8), approvals reuse the shipped host window (§6), and §8 says where each stage stands; see §1.1.
**Author:** Korp@narko, 2026-10-14, at the operator's request
**Trigger (operator, 2026-10-14):** "for agentmux users without claude's special gmail connector, how could any agentmux harness access a user's gmail? is that something we can add to connectors?" then "can we do all google services at once? Gmail, Calendar, and Drive .. spec them at once, but we can build them in stages. research best practices. Ideally in connectors we have the best practice 'connect...' we did some work with github before".
**Written against:** `main` @ `7b2448f87`.
**Builds on:** `archive/SPEC_TRUST_CENTER_2026_06_15.md` (the service-OAuth design, §4.2, §12), `SPEC_FIRST_CLASS_GITHUB_APP_AND_AWS_IDENTITY_2026_09_19.md` (the GitHub connect work and its findings), `SPEC_CONNECTORS_CARDS_2026_10_13.md` (cards), `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` (what agents launch with), `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` §5.4 and §5.6 (approval answered by the human, through the host).

---

## 1. Summary

1. **One "Connect Google" in Connectors → Accounts.** It signs in to a Google account and offers **Gmail, Calendar and Drive** as switches on that one connection. Each switch asks Google only for its own permissions, when it is turned on. Several Google accounts (personal and work) can be connected side by side.
2. **Agents get tools, never tokens.** srv keeps the tokens in the OS keychain and makes the Google API calls. Agents call three AgentMux MCP tools, `Calendar`, `Drive` and `Gmail`, each with actions (`Gmail` `search_threads`, `Calendar` `list_events`, `Drive` `read_file_content` …). That works the same for every harness that speaks MCP (Claude Code, Codex, Gemini, …), in containers and over SSH, and no agent can copy a mail token out. This deliberately differs from GitHub, whose token is put in the agent's environment (§2.3).
3. **The human decides who and what.** An agent reaches a Google service only if the account is bound to it and that service is allowed for it. Actions that can't be taken back, or that reach other people, wait for the human to approve them in an AgentMux window the agent can't click: sending mail, inviting people, sharing a file, deleting anything (§6).
4. **Built in stages ordered by Google's own rules.** Google ranks permissions as non-sensitive, sensitive and restricted (§4), and the higher tiers cost app review and, for restricted ones, a yearly paid security assessment. So: the OAuth core first (G0), then **Calendar** (sensitive, G1), then **Drive** through Google's file picker (non-sensitive, G2), then **Gmail** (restricted, G3), with the user's own OAuth client until AgentMux's is assessed.
5. **Compatible with Google's official MCP servers.** Google now hosts MCP servers for Gmail, Drive and Calendar, in developer preview (§3.4). The tools here use the stable REST APIs and mirror Google's tool names, so srv can later forward to Google's servers without agents noticing (G4).

### 1.1 What changed since this was written (2026-10-24)

- **G0 is built** (#5310, #5313). What is left of it is on the Google side: AgentMux's own Desktop client (W5) and its consent-screen scopes (Kanban #168), both console steps with the operator signed in. Creating OAuth clients and consent-screen entries is a console step, because Google has no public API for them.
- **MuxAPI tools are action-style** (since 2026-10-15: one tool per area with an `action`). The Google tools are three tools, `Calendar`, `Drive` and `Gmail`, whose actions are named after Google's own MCP tools, not about twenty separate tools (D3, §5).
- **The Vault exists** (`SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md`). It holds secrets agents may *use* with the person's approval. Connector tokens are not that: agents must never hold them. So tokens and the BYO client stay in the OS keychain, srv-only (D2, D8).
- **The host approval window is shipped and shared**: SSH host consent, `Agents spawn`, and the browser panes' attention and auto-approve (`browser_attention.rs`, `browser_bypass.rs`) all answer only through the host with the host token. G1's approvals reuse that path (§6).

## 2. What exists (verified on `main`)

### 2.1 The service-OAuth stack (shipped as code, usable only with your own client)

- **`crates/srv/src/identity/oauth_client.rs`** (#1443): Authorization Code with PKCE (S256) on a `127.0.0.1:0` loopback redirect for Google and Microsoft. Device Grant for GitHub. `state` is checked and the browser is the system browser, following RFC 8252/7636/8628/9700. Providers: `google` (`openid email profile`), `microsoft`, `github`, `slack`.
- **Every built-in `client_id` is `None`**, so a flow runs only with BYO credentials (`ByoCredentials`). BYO credentials are **not persisted**: the user retypes them each time. The Trust Center spec §12.1 planned keychain storage, which wasn't built.
- **Tokens:** `persist_oauth_account` puts `{access_token, refresh_token, expires_at}` as JSON in the keychain (`secret_store.rs`, service `agentmux`) and writes an `IdentityAccount{kind:"oauth"}` row. Gaps:
  - The Google request sends neither `access_type=offline` nor `prompt=consent`, so **Google never returns a refresh token**.
  - Nothing ever refreshes or **revokes** a token. Deleting the account only drops the keychain entry.
  - The account's email isn't captured, and the token isn't validated: `key_validator.rs` has no Google arm.
- **RPCs:** `account.oauth.start|poll|cancel` (`server/agent_handlers/identity.rs:446-492`).

### 2.2 The Connect UI (#1478, #1482, #4851)

- Connectors → Accounts shows brand tiles (`AccountsGallery`). A tile with both modes opens a chooser, "Connect with OAuth" or "Add API key / token". The OAuth path renders `OAuthConnectPanel`, which asks for a BYO client ID while `oauth-catalog.ts` has `builtIn: false`, then shows a device code (GitHub) or waits for the browser (PKCE).
- Google's tile is OAuth-only (`accounts-catalog.ts:37`, blurb "Workspace, Cloud"). The BYO hint tells the user to create a "Desktop app" client in the Google Cloud console.
- Connected accounts are cards (`account-card.tsx`) with a status dot and "Used by N agents". Details open under the card (`AccountDetailPanel`).

### 2.3 How a credential reaches an agent today

- An account is bound to an agent ("Bind to agent", `db_agent_identity_links`). At spawn, `resolver/inject.rs` turns bindings into environment variables by `provider_class` (`resolver/provider.rs`): GitHub becomes `GITHUB_TOKEN` and `GH_TOKEN`.
- **`google` (and `slack`) map to `None`.** The binding is logged and skipped, so a connected Google account reaches no agent. Today it is a dead end.
- **Latent bug, fixed in G0:** an OAuth-connected GitHub account stores the JSON blob, and the resolver injects the blob raw (`GH_TOKEN='{"access_token":…}'`). `IdentityValidate` sends the same raw blob as a Bearer token.

### 2.4 Pieces this spec reuses

- **The human-approval window:** `app_api/connections.rs` `ask_user(…)` asks the host, which opens an approval window (as for SSH host consent, `crates/cef/src/ssh_approval.rs`) that agents can't click and that fails closed when no UI is up. The browser-pane approval follows the same rule: the answer reaches srv only from the host, with the host token (`browser_attention.rs`, spec §5.6).
- **MCP tools:** a schema in `crates/mcp/src/tool_schemas.rs`, an entry in `main.rs`'s `tools/list`, a family file under `crates/mcp/src/tools/` (as `accounts.rs` does: an srv route called with the agent's verified identity), and a route in `server/routes.rs`.
- **Agent identity on srv routes:** `actor::check_actor` / `SelfOwner` give the verified calling agent, which per-agent grants need.

## 3. Best practice, researched 2026-10-14

### 3.1 Google OAuth for a desktop app

- **Client type "Desktop app"**, system browser, loopback redirect `http://127.0.0.1:<port>`, PKCE S256 on every request ([OAuth 2.0 for iOS & Desktop Apps](https://developers.google.com/identity/protocols/oauth2/native-app)). The loopback method is deprecated for Android, iOS and Chrome app clients, **not for desktop**. Desktop clients "cannot keep secrets": a client secret embedded in the app is not treated as confidential.
- **Refresh tokens:** `access_type=offline`. The OIDC `offline_access` scope alone is not enough ([Workspace MCP configuration](https://developers.google.com/workspace/guides/configure-mcp-servers)). There are limits per client and user, and per user across clients.
- **Incremental authorization:** ask for scopes when the feature needs them, with `include_granted_scopes=true` so each new token covers everything granted so far ([best practices](https://developers.google.com/identity/protocols/oauth2/resources/best-practices)).
- **Granular consent is always on** for new clients: the user can untick any scope. **Read the `scope` field of the token response** and turn off what wasn't granted ([granular permissions](https://developers.google.com/identity/protocols/oauth2/resources/granular-permissions)).
- **Revoke on disconnect:** `POST https://oauth2.googleapis.com/revoke`. This "removes all OAuth 2.0 scopes previously granted to a project".
- **Refresh tokens can die** without warning: revoked by the user, unused for six months, the per-client limit (the oldest is dropped), a Workspace admin policy, a password change for Gmail scopes, or the 7-day expiry of an app still in Testing. A refresh that returns `invalid_grant` means "reconnect", not "retry".
- **Newer:** Google's best-practices page recommends sender-constrained tokens (DPoP) for refresh tokens. Noted, not in the first stages.

### 3.2 Permission tiers decide the build order

| Tier | Google requires | Scopes here |
|---|---|---|
| Non-sensitive | brand verification only | `openid email profile`; `drive.file` |
| Sensitive | sensitive-scope verification (review, demo video) | Calendar: `calendar.events.readonly`, `calendar.calendarlist.readonly`, `calendar.events.freebusy`, `calendar.events` |
| Restricted | restricted-scope verification **plus a yearly CASA security assessment** by an approved lab, if the data can reach a server | Gmail: `gmail.readonly`, `gmail.compose` (and every other Gmail scope); Drive: `drive.readonly`, `drive` |

Sources: [sensitive scope verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification), [Calendar scopes](https://developers.google.com/workspace/calendar/api/auth), [Drive scopes](https://developers.google.com/workspace/drive/api/guides/api-specific-auth) (it marks `drive.file` "Recommended / Non-sensitive" and `drive.readonly` "Restricted"), [Gmail restricted scopes and CASA](https://developer.nylas.com/docs/provider-guides/google/google-verification-security-assessment-guide/). Third-party figures put CASA Tier 2 at $15K–$75K a year. Treat that as a rough range.

Exemptions that matter here:
- An app in **Testing** serves up to 100 listed test users, and their grants expire after 7 days.
- An app the user owns (**BYO**) and publishes "In production" without verification keeps long-lived tokens. Users click through an "unverified app" warning.
- A Workspace organization's **Internal** app has no review, no 100-user cap and no 7-day expiry, for users in that organization.
- Whether a desktop app whose mail never touches the developer's servers still needs CASA is **unclear**. Google's wording keys the assessment on access "from or through a third-party server", and a community answer says it applies regardless beyond 100 users. This is an owner decision, informed by asking Google (§11 Q1).

### 3.3 Drive without the restricted scopes: the desktop Picker

- Google's file picker now works for desktop apps through OAuth parameters: add `trigger_onepick=true` and `prompt=consent` to the authorization request. Google opens the picker in the system browser and redirects back with `picked_file_ids` and a code ([Picker for desktop and mobile apps](https://developers.google.com/workspace/drive/picker/guides/desktop-mobile-picker)). Optional: `allow_multiple`, `mimetypes`, `allow_folder_selection`.
- **Only `drive.file` is allowed on that request, and it can't be combined with other scopes.** So the picker is its own authorization request, separate from the main connection's. Whether that also means a separate OAuth client is to be checked in G2's spike (§11 Q4).
- `drive.file` reaches files the user picked and files the app created. It can't search the whole Drive: that needs `drive.readonly` (restricted).
- **As built (G2):** the picker is its own request on the account's own client (the one its token blob records), `drive.file` alone with `trigger_onepick=true`, `prompt=consent` and `allow_multiple=true`, and neither `include_granted_scopes` nor `access_type`. The callback's `picked_file_ids` come back to the app, a `code` that comes with them is exchanged and dropped (the account keeps its own tokens), and the files are then reachable with the account's existing `drive.file` token, so `search_files` lists them: there is no separate file list to keep. Drive has to be turned on for the account first (`drive.file` granted). The request carries the account's email as `login_hint`, and after the code exchange the authorizing user is read from Drive's `about` and compared with the account's email (a mismatch fails with "pick again as …"; an answer Google doesn't give skips the check). The callback's request line is read whole (up to 32 KiB), since a pick of 100 files is longer than 4 KiB. These points are Google's behaviour that the dev spike confirms (the first three) and one more (that `about` answers to `drive.file`): that the callback carries `picked_file_ids` as a comma-separated list, that the picker works without `include_granted_scopes`, and that a pick made in one request is visible to the token from another.

### 3.4 Google's official Workspace MCP servers

- Google hosts remote MCP servers per product over Streamable HTTP with OAuth 2.0 ([configure the Workspace MCP servers](https://developers.google.com/workspace/guides/configure-mcp-servers)): `gmailmcp.googleapis.com/mcp/v1`, `calendarmcp…`, `drivemcp…`. The OAuth client must be marked "Use this client for an AI-powered agent". Each product needs its API and its MCP API enabled.
- **Status: Developer Preview** (rollout announced from 2026-05-01: [Workspace updates](https://workspaceupdates.googleblog.com/2026/05/agent-tools-and-security-updates-for-workspace-developers.html)).
- Their scopes are the same tiers: Gmail `gmail.readonly` and `gmail.compose`; Drive `drive.readonly` and `drive.file`; Calendar read-only scopes. Their tools include Gmail `search_threads`, `get_thread`, `create_draft`, `list_labels`, `label_message`; Calendar `list_events`, `search_events`, `create_event`, `update_event`, `respond_to_event`; Drive `search_files`, `read_file_content`, `create_file`, `copy_file`, `get_file_metadata`.
- **Why not point agents straight at them now:** they are a preview. The user's token would have to be written into each agent's MCP config, where any agent could read it. And there would be no per-agent grants or approval, which their own docs ask for ("carefully review the actions taken by your AI client").

### 3.5 Patterns from other agent products

- Connect once per provider, then choose services. Show what was granted, not what was asked.
- Read-only by default. Writes and sends behind a confirmation the model can't give itself.
- Treat mail and documents as untrusted input (prompt injection). Google's MCP docs warn about exactly this.

## 4. Decisions

- **D1. One connection, three services.** A Google account is one card. Gmail, Calendar and Drive are switches on it, each with a read level and a write level, not three accounts. A second Google account is a second card.
- **D2. srv holds the tokens and makes the calls; not the Vault.** (The Vault is for secrets an agent uses, with the person's say-so; a connector token is one no agent ever holds, so it stays in the keychain, srv-only.) The refresh token stays in the keychain. Access tokens live only in srv's memory, refreshed when they near expiry. No token goes into an agent's environment, `.mcp.json`, an ADC file, a log or a tool result. `provider_class` keeps returning nothing to inject for `google`. *Why not GitHub's way:* a mail token is the highest-value credential a user has, mail content is the main prompt-injection channel, and an agent holding the raw token can't be limited, audited or asked to wait for approval.
- **D3. Three AgentMux MCP tools, their actions named after Google's.** `Calendar`, `Drive` and `Gmail` in `agentmux-mcp`, each with an `action` (as `Tasks`, `Vault` and the rest are since 2026-10-15), each action a thin call to an srv route, and each action named after the Google MCP tool it matches (§5), so G4 can forward to Google's servers behind the same names. They reach every harness and every agent location (host, container, SSH) that already reaches `agentmux-mcp`. A tool appears only when the agent has a grant for that service (D4); otherwise it isn't in the agent's tool list at all, which keeps the list short for everyone else.
- **D4. Per-agent grants, default none.** An agent sees a Google tool only if the account is bound to it ("Bind to agent", existing) **and** that service is allowed for it at a level: `off`, `read`, or `read + write`. Grants live in srv, keyed by verified agent identity (the definition id behind the agent's token). A tool call from an agent without the grant fails with a sentence saying who can grant it. Grants change only through the host, as the Vault's writes do (`/api/v1/host/google`, with the host's IPC token): agents hold srv's instance key, so a grant settable over that key could be granted by an agent to itself.
- **D5. The human approves what can't be undone or reaches other people** (§6), through the host approval window (§2.4), never in the agent's chat.
- **D6. Content from Google is page content.** Tool results that carry mail bodies, event descriptions or file text are marked as untrusted, as `BrowserSnapshot` results are, and the tool descriptions tell the agent not to follow instructions found in them.
- **D7. Every call is logged per account.** Time, agent, tool, the ids touched, and outcome (no content). Shown in the account's detail panel. Kept for 30 days.
- **D8. Three ways to supply the OAuth client.**
  - **Built-in:** AgentMux's own client, the one-click path once verified.
  - **BYO:** the user's own Desktop client ID and secret, now **saved in the keychain** (built, G0a). Not a Vault entry: the client secret is used by srv for the token exchange, never by an agent, and a Vault entry is something an agent can be granted.
  - **Workspace internal:** the same as BYO, created as "Internal" in the user's organization.
  The connection records which one it uses. Each service says which client modes can turn it on (§7).

## 5. Tools

Three tools, each with an `action`. Every action takes an optional `account` (the account's email) when several are bound. Read actions page and cap their output. Results that carry content from Google are wrapped as untrusted page content (D6).

| Tool | Read actions | Write actions (approval per §6) |
|---|---|---|
| `Calendar` | `list_calendars`, `list_events`, `search_events`, `free_busy` | `create_event`, `update_event`, `respond_to_event`, `delete_event` |
| `Drive` | `search_files` (picked and app-created files under `drive.file`), `get_file_metadata`, `read_file_content` (text export for Docs, Sheets and Slides; size-capped) | `create_file`, `update_file`, `copy_file`, `share_file`, `trash_file`; `pick_files` asks the person to pick files (opens the picker for them, returns ids) |
| `Gmail` | `search_threads` (Gmail query syntax), `get_thread`, `list_labels` | `create_draft`, `update_draft`, `label_thread`, `send_draft`, `trash_thread` |

The action names are Google's own MCP tool names where Google has one (`list_events`, `search_events`, `create_event`, `update_event`, `respond_to_event`; `search_files`, `read_file_content`, `create_file`, `copy_file`, `get_file_metadata`; `search_threads`, `get_thread`, `list_labels`, `create_draft`, `label_message` → `label_thread`).

- **No raw send.** An agent writes a draft. Sending is `Gmail` `send_draft`, which always asks (§6), and the approval window shows the draft exactly as it will be sent. A draft also lets the human fix it in Gmail first.
- **Attachments:** `Drive` `read_file_content` and `Gmail` `get_thread` return attachment metadata. Downloading an attachment saves it into the calling agent's workspace (size-capped) and returns the path, never into the tool result.

## 6. What waits for the human

The approval reuses the shipped host path (§1.1): srv asks the host, the host shows the window, and the answer reaches srv only with the host token; no agent route, `UI` click or `Browser` action can answer it. The window names the agent, the account and the action in words, with the exact payload (recipients, subject and body for mail; attendees and time for events; file and new access for sharing). Answers are **Allow once** and **Deny**. There is also **Always allow for this agent**, offered only for the low-risk rows. A request nobody answers fails after two minutes (the host window's own limit). Agents never see or click the window. Its answer reaches srv only from the host (§2.4).

| Action | Asks |
|---|---|
| `Gmail` `send_draft` | always |
| `Gmail` `trash_thread`, `Drive` `trash_file`, `Calendar` `delete_event` | always |
| `Drive` `share_file` (any new access), or creating a public link | always |
| `Calendar` `create_event` / `update_event` / `respond_to_event` that **notifies other people** (attendees, invitations, replies to the organizer) | always |
| `Calendar` `create_event` / `update_event` on the user's own calendar with no other attendees | first time per agent, then "Always allow" may be chosen |
| `Gmail` `create_draft`, `update_draft`, `label_thread`; `Drive` `create_file`, `update_file`, `copy_file` | no, if the agent has `read + write` |
| All reads | no, if the agent has `read` |

Rate limits per agent and account (for example, at most 20 sends a day) are a backstop in case a tricked agent asks over and over.

## 7. The Connect experience

**Connect (new account)**
1. Connectors → Accounts → **Google** tile → **Connect Google**. If no built-in client is available, the panel shows the BYO fields once, with the Cloud console steps (§2.2), and saves them.
2. The system browser opens Google's sign-in with `openid email profile` only. It returns to the loopback listener (§2.1), with a 5-minute timeout and a Cancel button.
3. The card appears with the account's **email** (from the ID token) and a picture tile, with no services yet. srv validates the token (`tokeninfo` / userinfo), which fills the missing Google arm of `key_validator.rs`.

**Turn a service on**
- In the card's details, each service has a switch and a level (`read`, `read + write`), with one line on what it lets agents do and which Google tier it is.
- Turning one on runs an incremental authorization for just that service's scopes (`include_granted_scopes=true`, `access_type=offline`, `prompt=consent` the first time). Then it reads back the granted scopes:
  - All granted: the service shows **On**.
  - Some unticked: the service shows **Partly granted: <what's missing>** and its tools are limited to what was granted.
- A service the current client mode can't offer (Gmail on the built-in client before verification) shows why and the way through ("use your own Google client" or "Workspace internal app").

**Give agents access**
- The details list the agents bound to the account, each with a per-service level (D4). "Bind to agent" stays the way to add one.
- Grants are checked on every call, so removing one stops a running agent at once, with a sentence. The tool *list* is read when the agent's MCP starts (`/api/v1/google/self/services`): a service granted to a running agent shows its tool from the agent's next launch.

**Status on the card**
- `Connected` (green): token refreshes.
- `Reconnect needed` (warn): `invalid_grant`, with a Reconnect button that reruns the consent for the services that were on.
- `Blocked by your organization` (error): Google's `admin_policy_enforced`, with the admin's next step.
- `Using your own client` / `Workspace internal` shown as a tag.

**Disconnect**
- Calls Google's revoke endpoint, then deletes the keychain entry, the grants and the log. If revoking fails (offline), it says so and offers a retry, because the grant would otherwise live on at Google.

**Pick Drive files**
- **"Give agents files from Drive…"** in the Drive section opens the desktop picker (§3.3) in the system browser. Picked ids join the account's file list, which agents with Drive `read` can see.
- `Drive` `pick_files` lets an agent ask for this. It opens the same picker for the human, never for the agent.

## 8. Stages

Where each stands, 2026-10-28: **G0 built** in code (#5310, #5313); its Google side (W5's Desktop client, the consent-screen scopes, #168) is open. **G1a is built** (Calendar reads: grants and their panel, the `Calendar` tool's four read actions, the per-account log); **G1b** (Calendar writes with approvals) is next. **G2's reads are built**: the `Drive` tool's three read actions and `pick_files` (opens the picker for the person, answers at once, at most one per account a minute), Drive as a second service in the grants panel, and the "Give agents files from Drive…" button. Drive's write actions wait for the approvals of G1b. **G3's reads are built**: `search_threads` (Gmail's own query syntax, with the subject, sender and date of the first ten threads), `get_thread` (each message as text, HTML mail as plain text, attachments listed by name, type and size and never downloaded; a message is cut at 20,000 characters, a thread at 30 messages and 120,000), `list_labels`, a Gmail row in the grants panel, and a refusal to turn Gmail on with AgentMux's own client (the sentence says to connect with your own Google client). The log holds the action and thread ids, never the search words or the mail. **G3's writes are built**: a Gmail write level (the person's "Read and write" for an agent, after allowing writing on the account, which asks `gmail.compose` and `gmail.modify` and nothing more); `create_draft` and `update_draft` build the mail as RFC 5322 text (recipients checked as plain addresses, no header can be injected, a non-ASCII subject encoded) and need no approval; `label_thread` needs none either, but cannot add the trash label; `send_draft` and `trash_thread` ask through the host window (two minutes to answer; Send / Don't send, Move to trash / Keep it). The send window shows every recipient (Bcc too), the subject and the text (cut at 3,000 characters), and the draft is read again after the yes: a draft that changed in between is not sent. At most 20 sends a day and 10 questions an hour per agent and account, counted from the account's log. Not built: `Gmail` attachment downloads into the workspace (G3's last item). Until W5 exists, every stage is built and tested with a BYO client in a test project, created there by the operator (§11 Q6).

| Stage | Ships | Google requirements |
|---|---|---|
| **G0. OAuth core and the Google account** | `access_type=offline`, `prompt=consent`, `include_granted_scopes`; refresh in srv; revoke on delete; granted-scope read-back; account email; Google validator; BYO credentials saved in the keychain; token-blob parsing (fixes §2.3's GitHub bug); the Connect panel and card states of §7; register AgentMux's Desktop client and its consent screen (`agentmux.ai` homepage, privacy policy, domain verified). | Brand verification (non-sensitive scopes only) |
| **G1. Calendar** | The `Calendar` tool; grants (D4) and the per-agent tool list; the approval wiring (§6) and the per-account log, which every later stage reuses. Read first (G1a), then write (G1b). | Sensitive-scope verification for the built-in client (consent screen, scope justification, demo video); BYO meanwhile |
| **G2. Drive** | The `Drive` tool on `drive.file`; the desktop picker spike and then `pick_files` and "Give agents files…"; Docs/Sheets/Slides text export. | None beyond G0 for `drive.file` |
| **G3. Gmail** | The `Gmail` tool with drafts-then-send; attachments to the workspace; rate limits. **BYO and Workspace-internal clients only** at first. | For the built-in client: restricted-scope verification plus CASA, per the owner's decision (§11 Q1) |
| **G4. Later** | Forward to Google's official MCP servers once they leave preview, behind the same tools, grants and approvals. Full-Drive search (`drive.readonly`, restricted). DPoP for refresh tokens. | Restricted for `drive.readonly` |

Each stage is one or two PRs, tested against a real Google account on a test project before review.

## 9. Security

1. **Prompt injection is the main threat.** A crafted email or document tells the agent to forward mail, share a file or invite someone. Mitigations, in order of weight:
   - every action that sends, shares, invites or deletes waits for the human (§6);
   - agents get only the services and levels granted (D4);
   - content is marked untrusted (D6);
   - per-agent rate limits;
   - the log shows what happened (D7).
2. **No token leaves srv** (D2). A compromised or curious agent can make the calls its grants allow and nothing more. It can't take the credential elsewhere.
3. **The approval can't be forged by an agent.** It is answered only through the host with its token (§2.4). srv's ordinary auth key, which agents hold, can't answer it.
4. **Same-OS-user residual:** a process running as the user can read the keychain entry. That is machine compromise, the same residual as every other connected account.
5. **Data handling:** mail, events and files go only from Google to srv to the agent, then to whichever model provider that agent uses. AgentMux runs no server in this path. The consent screen, the privacy policy and the Connect panel say so plainly. Google's user-data policy (Limited Use, including its AI rules) applies, and the wording is reviewed before verification (§11 Q2).
6. **Logs hold ids and outcomes, never content or tokens.**

## 10. Tests

- **Unit (srv):**
  - building the authorization URL (all parameters, incremental scopes);
  - reading back granted scopes, including partial grants;
  - refresh near expiry;
  - `invalid_grant` leading to "reconnect";
  - revoke on delete, and its failure path;
  - token-blob parsing (the GitHub case too);
  - grant checks per agent, service and level;
  - the approval matrix of §6 per tool and argument (attendees vs none);
  - rate limits;
  - untrusted marking on results;
  - redaction (no content or token in logs).
- **MCP:** schemas registered in `tools/list` (the family test), errors worded for an agent.
- **Frontend:** card states, service switches with partial grants, the per-agent level editor, Disconnect's retry.
- **End to end, per stage, against a test Google account:**
  - connect, then turn the service on;
  - an agent reads;
  - an agent drafts or creates;
  - a send, invite or share waits for approval: Deny leaves nothing behind, Allow does exactly what was shown;
  - disconnect revokes (verified at myaccount.google.com/permissions);
  - a second account side by side;
  - a Workspace account whose admin blocks the app shows the "blocked" state.

## 11. Open questions

1. **Gmail on AgentMux's own client:** pay for restricted-scope verification and a yearly CASA assessment, or stay BYO/Workspace-internal for Gmail? Ask Google first whether a desktop app with no server in the data path is in scope for CASA (§3.2).
2. **Limited Use and model providers:** confirm with Google's policy text (and counsel) how to describe passing mail content to the user's chosen AI provider in the consent screen and privacy policy.
3. **Microsoft 365 next?** `oauth_client.rs` already has a Microsoft entry. Outlook mail, calendar and OneDrive fit the same shape. Out of scope here.
4. **Picker client:** does the desktop picker's "only `drive.file`, not combined" rule need a separate OAuth client, or just a separate request? G2 is built as a separate request on the account's own client; the dev spike (§3.3) confirms whether Google accepts that, and the answer is recorded here.
5. **Defaults:** should "read" on a service be offered to every bound agent automatically, or stay off until set per agent? Proposal: off (D4).
6. **A test client until W5:** the operator creates a Desktop OAuth client in a test project (console, a few minutes), with a test user, so G1–G3 can be built and tested against a real Google account before AgentMux's own client and its verification exist. Its id and secret go into the BYO fields (the keychain), never into a repository.
7. **`gmail.modify` for labels and trash (decided by the build, for the owner to confirm):** `label_thread` and `trash_thread` need `gmail.modify`, a restricted scope wider than `gmail.readonly` and `gmail.compose`. It is asked only when the person allows Gmail writing on the account, and only on a client they brought, like the rest of Gmail. If the owner would rather not ask Google's review for it, those two actions go and drafts and sending stay.
