# SPEC: Google in Connectors: Gmail, Calendar and Drive for every agent

**Status:** active — G0's srv half is built in #5310 (G0a, 2026-10-21): the Google authorization parameters, incremental scopes on an existing account, granted-scope read-back, the email from the ID token, refresh in srv (`needs_reauth` on `invalid_grant`), revoke on delete, the Google validator, token-blob parsing (§2.3's GitHub bug) and the BYO client saved in the keychain (`identity/oauth_client.rs`, `identity/oauth_tokens.rs`). G0's frontend half (the Connect panel and card states of §7), registering AgentMux's own client, and G1–G4 follow.
**Author:** Korp@narko, 2026-10-14, at the operator's request
**Trigger (operator, 2026-10-14):** "for agentmux users without claude's special gmail connector, how could any agentmux harness access a user's gmail? is that something we can add to connectors?" then "can we do all google services at once? Gmail, Calendar, and Drive .. spec them at once, but we can build them in stages. research best practices. Ideally in connectors we have the best practice 'connect...' we did some work with github before".
**Written against:** `main` @ `7b2448f87`.
**Builds on:** `archive/SPEC_TRUST_CENTER_2026_06_15.md` (the service-OAuth design, §4.2, §12), `SPEC_FIRST_CLASS_GITHUB_APP_AND_AWS_IDENTITY_2026_09_19.md` (the GitHub connect work and its findings), `SPEC_CONNECTORS_CARDS_2026_10_13.md` (cards), `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` (what agents launch with), `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` §5.4 and §5.6 (approval answered by the human, through the host).

---

## 1. Summary

1. **One "Connect Google" in Connectors → Accounts.** It signs in to a Google account and offers **Gmail, Calendar and Drive** as switches on that one connection. Each switch asks Google only for its own permissions, when it is turned on. Several Google accounts (personal and work) can be connected side by side.
2. **Agents get tools, never tokens.** srv keeps the tokens in the OS keychain and makes the Google API calls. Agents call AgentMux MCP tools (`GmailSearchThreads`, `CalendarListEvents`, `DriveReadFile` …). That works the same for every harness that speaks MCP (Claude Code, Codex, Gemini, …), in containers and over SSH, and no agent can copy a mail token out. This deliberately differs from GitHub, whose token is put in the agent's environment (§2.3).
3. **The human decides who and what.** An agent reaches a Google service only if the account is bound to it and that service is allowed for it. Actions that can't be taken back, or that reach other people, wait for the human to approve them in an AgentMux window the agent can't click: sending mail, inviting people, sharing a file, deleting anything (§6).
4. **Built in stages ordered by Google's own rules.** Google ranks permissions as non-sensitive, sensitive and restricted (§4), and the higher tiers cost app review and, for restricted ones, a yearly paid security assessment. So: the OAuth core first (G0), then **Calendar** (sensitive, G1), then **Drive** through Google's file picker (non-sensitive, G2), then **Gmail** (restricted, G3), with the user's own OAuth client until AgentMux's is assessed.
5. **Compatible with Google's official MCP servers.** Google now hosts MCP servers for Gmail, Drive and Calendar, in developer preview (§3.4). The tools here use the stable REST APIs and mirror Google's tool names, so srv can later forward to Google's servers without agents noticing (G4).

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
- **D2. srv holds the tokens and makes the calls.** The refresh token stays in the keychain. Access tokens live only in srv's memory, refreshed when they near expiry. No token goes into an agent's environment, `.mcp.json`, an ADC file, a log or a tool result. `provider_class` keeps returning nothing to inject for `google`. *Why not GitHub's way:* a mail token is the highest-value credential a user has, mail content is the main prompt-injection channel, and an agent holding the raw token can't be limited, audited or asked to wait for approval.
- **D3. AgentMux MCP tools, named after Google's.** `Gmail*`, `Calendar*` and `Drive*` families in `agentmux-mcp`, each a thin call to an srv route, named so each maps one-to-one to Google's MCP tool (§5). They reach every harness and every agent location (host, container, SSH) that already reaches `agentmux-mcp`.
- **D4. Per-agent grants, default none.** An agent sees a Google tool only if the account is bound to it ("Bind to agent", existing) **and** that service is allowed for it at a level: `off`, `read`, or `read + write`. Grants live in srv, keyed by verified agent identity. A tool call from an agent without the grant fails with a sentence saying who can grant it.
- **D5. The human approves what can't be undone or reaches other people** (§6), through the host approval window (§2.4), never in the agent's chat.
- **D6. Content from Google is page content.** Tool results that carry mail bodies, event descriptions or file text are marked as untrusted, as `BrowserSnapshot` results are, and the tool descriptions tell the agent not to follow instructions found in them.
- **D7. Every call is logged per account.** Time, agent, tool, the ids touched, and outcome (no content). Shown in the account's detail panel. Kept for 30 days.
- **D8. Three ways to supply the OAuth client.**
  - **Built-in:** AgentMux's own client, the one-click path once verified.
  - **BYO:** the user's own Desktop client ID and secret, now **saved in the keychain**.
  - **Workspace internal:** the same as BYO, created as "Internal" in the user's organization.
  The connection records which one it uses. Each service says which client modes can turn it on (§7).

## 5. Tools

All take an optional `account` (the account's email) when several are bound. Read tools page and cap their output.

| Service | Read | Write (approval per §6) | Google MCP equivalent |
|---|---|---|---|
| Calendar | `CalendarListCalendars`, `CalendarListEvents`, `CalendarSearchEvents`, `CalendarFreeBusy` | `CalendarCreateEvent`, `CalendarUpdateEvent`, `CalendarRespondToEvent`, `CalendarDeleteEvent` | `list_events`, `search_events`, `create_event`, `update_event`, `respond_to_event` |
| Drive | `DriveSearchFiles` (picked and app-created files under `drive.file`), `DriveGetFileMetadata`, `DriveReadFile` (text export for Docs, Sheets and Slides; size-capped) | `DriveCreateFile`, `DriveUpdateFile`, `DriveCopyFile`, `DriveShareFile`, `DriveTrashFile`; `DrivePickFiles` asks the user to pick files (opens the picker, returns ids) | `search_files`, `read_file_content`, `create_file`, `copy_file`, `get_file_metadata` |
| Gmail | `GmailSearchThreads` (Gmail query syntax), `GmailGetThread`, `GmailListLabels` | `GmailCreateDraft`, `GmailUpdateDraft`, `GmailLabelThread`, `GmailSendDraft`, `GmailTrashThread` | `search_threads`, `get_thread`, `list_labels`, `create_draft`, `label_message` |

- **No `GmailSend` of raw text.** An agent writes a draft. Sending is `GmailSendDraft`, which always asks (§6), and the approval window shows the draft exactly as it will be sent. A draft also lets the human fix it in Gmail first.
- **Attachments:** `DriveReadFile` and `GmailGetThread` return attachment metadata. Downloading an attachment saves it into the calling agent's workspace (size-capped) and returns the path, never into the tool result.

## 6. What waits for the human

The approval window names the agent, the account and the action in words, with the exact payload (recipients, subject and body for mail; attendees and time for events; file and new access for sharing). Answers are **Allow once** and **Deny**. There is also **Always allow for this agent**, offered only for the low-risk rows. A request nobody answers fails after 10 minutes. Agents never see or click the window. Its answer reaches srv only from the host (§2.4).

| Action | Asks |
|---|---|
| `GmailSendDraft` | always |
| `GmailTrashThread`, `DriveTrashFile`, `CalendarDeleteEvent` | always |
| `DriveShareFile` (any new access), or creating a public link | always |
| `CalendarCreateEvent` / `UpdateEvent` / `RespondToEvent` that **notifies other people** (attendees, invitations, replies to the organizer) | always |
| `CalendarCreateEvent` / `UpdateEvent` on the user's own calendar with no other attendees | first time per agent, then "Always allow" may be chosen |
| `GmailCreateDraft`, `GmailUpdateDraft`, `GmailLabelThread`, `DriveCreateFile`, `DriveUpdateFile`, `DriveCopyFile` | no, if the agent has `read + write` |
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
- Changes reach a running agent without a restart, because tools are checked per call. The tool *list* an agent saw at launch doesn't change: a tool it may not use fails with a sentence.

**Status on the card**
- `Connected` (green): token refreshes.
- `Reconnect needed` (warn): `invalid_grant`, with a Reconnect button that reruns the consent for the services that were on.
- `Blocked by your organization` (error): Google's `admin_policy_enforced`, with the admin's next step.
- `Using your own client` / `Workspace internal` shown as a tag.

**Disconnect**
- Calls Google's revoke endpoint, then deletes the keychain entry, the grants and the log. If revoking fails (offline), it says so and offers a retry, because the grant would otherwise live on at Google.

**Pick Drive files**
- **"Give agents files from Drive…"** in the Drive section opens the desktop picker (§3.3) in the system browser. Picked ids join the account's file list, which agents with Drive `read` can see.
- `DrivePickFiles` lets an agent ask for this. It opens the same picker for the human, never for the agent.

## 8. Stages

| Stage | Ships | Google requirements |
|---|---|---|
| **G0. OAuth core and the Google account** | `access_type=offline`, `prompt=consent`, `include_granted_scopes`; refresh in srv; revoke on delete; granted-scope read-back; account email; Google validator; BYO credentials saved in the keychain; token-blob parsing (fixes §2.3's GitHub bug); the Connect panel and card states of §7; register AgentMux's Desktop client and its consent screen (`agentmux.ai` homepage, privacy policy, domain verified). | Brand verification (non-sensitive scopes only) |
| **G1. Calendar** | Calendar tools; grants; the approval window wiring (§6) and the per-account log, which every later stage reuses. Read first, then write. | Sensitive-scope verification for the built-in client (consent screen, scope justification, demo video); BYO meanwhile |
| **G2. Drive** | `drive.file` tools; the desktop picker spike and then `DrivePickFiles` and "Give agents files…"; Docs/Sheets/Slides text export. | None beyond G0 for `drive.file` |
| **G3. Gmail** | Gmail tools with drafts-then-send; attachments to the workspace; rate limits. **BYO and Workspace-internal clients only** at first. | For the built-in client: restricted-scope verification plus CASA, per the owner's decision (§11 Q1) |
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
4. **Picker client:** does the desktop picker's "only `drive.file`, not combined" rule need a separate OAuth client, or just a separate request? G2's spike answers it.
5. **Defaults:** should "read" on a service be offered to every bound agent automatically, or stay off until set per agent? Proposal: off (D4).
