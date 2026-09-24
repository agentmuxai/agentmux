# SPEC: show the provider account's email in the Armory

**Date:** 2026-09-23
**Status:** active — the capture-and-render path shipped in #3541, but
Claude's login transcript prints no email, so no Claude account ever had
one recorded and the Accounts page kept showing `claude-oauth`. The
follow-up reads the email the CLI records in the account's own config dir
(`<dir>/.claude.json` → `oauthAccount.emailAddress`): at login when the
transcript had none, and as §4's backfill each time accounts are listed
(written only when it differs, so a re-login as another user replaces it).
The row's label is now the email, falling back to the name
(`accountLabel`).
**Trigger:** Repo owner: *"for provider logins (like anthropic) we need the
email address on the account to show on its entry in the armory."*
**Scope:** OAuth provider accounts (`kind = "oauth"`). Static key/token
accounts have no login identity to show and are out of scope.

---

## 1. Why this is small

Every piece but one already exists. Verified against `main` @ `c676193`, and
against live data rather than inferred:

| Piece | State |
|---|---|
| Email captured at login | **Yes** — `identity/auth_session.rs` scrapes it from the login transcript into `captured_email`, surfaced as `AuthSessionStatus::Success { email, … }` |
| Email available on demand | **Yes** — `server/cli_handlers.rs:745` parses `claude auth status --json`; its comment notes *"Claude outputs `emailAddress`; other CLIs use `email`. Check both."* |
| A place to store it | **Yes** — `db_accounts.context` is a JSON blob (`migrations.rs:565`) |
| A row to render it on | **Yes** — `identity-accounts-tab.tsx:146` already renders `display_name` in `.identity-row-meta` when non-empty |
| Precedent for showing an email | **Yes** — the AgentMux Cloud row directly above shows `muxbus.status()?.email` (`accounts-manager.tsx:105`) |
| **The email being persisted** | **No** — this is the entire gap |

`server/identity_auth_persist.rs:64-66` builds the account with:

```rust
display_name: String::new(),
context: serde_json::json!({}),
```

Confirmed on live data: every real `db_accounts` row on this host reads
`display_name='' context={}`. The email is captured, used to confirm the login,
and dropped.

## 2. Where to store it

**`context.email`, not `display_name`.**

`display_name` is tempting because it renders with zero frontend work. It is
the wrong home:

- It is a **user-facing label**. If a user later names an account *"work"* or
  *"billing"*, writing the email there means either clobbering their label or
  refusing to record the email. Both are bad, and the conflict is permanent.
- The email is **provider account metadata**, which is what `context` is for.
- Keeping them separate means the email survives any renaming, and stays
  available to anything else that needs to identify the account — audit
  output, a future account picker, diagnostics.

So:

```json
context: { "email": "user@example.com" }
```

`display_name` remains free for the user.

## 3. What renders

*(Revised after #3541: the email in the muted meta line was not what the
repo owner wanted — the row still read `claude-oauth`.)* The row's **label**
is `accountLabel(a)`: `context.email` when present, else the account name.
The generic name moves to the label's tooltip; a user-set `display_name`
still shows in `.identity-row-meta` beside it:

```
user@example.com    work    ●
```

The same label names the account in its detail modal title, its delete
confirmation, the agent's bind-account picker and failure-row
"Bind: <account>" action, and the Armory's Bind-to-Agent menu — every
Claude account is named `claude-oauth`, so the name alone tells none of
them apart.

## 4. Existing accounts must backfill

Anyone already logged in has `context={}` and will never re-run the login path,
so a write-on-login-only fix leaves every current account blank — the most
visible case for the person who asked for this.

`confirm_authenticated` already runs `auth status --json` on the validation
path and already parses the email out (§1). Backfill there: when a probe
returns an email and the stored `context.email` is absent or different, write
it.

**Different, not just absent** — an account can be re-authenticated as a
different user, and a stale email displayed next to a live credential is worse
than no email at all.

## 5. Provider coverage

`cli_handlers.rs:745` already reads both `emailAddress` (Claude) and `email`
(others), so the parse is provider-agnostic today. Providers whose CLI reports
no email simply leave `context.email` absent and render as they do now.

**Non-goal:** fetching an email from a provider API that the CLI does not
already report. If the login does not surface one, this spec does not invent a
way to get one.

## 6. Privacy

An email address is personal data and this makes it more visible than before.

- It is already on the user's own machine in the provider's credential files;
  this surfaces what is already local, it does not transmit anything.
- It must not be written to logs at `info` or above. §4's backfill logs at
  `debug` with the value elided, and the existing `auth_session.rs` handling
  already treats the transcript as sensitive.
- It is not added to any wire payload that leaves the host. `context` already
  crosses the local RPC boundary to the frontend, which is the same trust
  boundary the credential itself lives behind.

## 7. Testing

- `identity_auth_persist` writes `context.email` when the session captured one
  and otherwise falls back to the config dir; it omits the key entirely when
  neither has one — **not** an empty string, so "absent" and "blank" cannot be
  confused downstream.
- `account_email`: reads Claude's recorded email (and the `claude-code`
  alias), skips the ambient `~/.claude`; the refresh backfills a blank
  account, replaces a stale email, leaves a matching one and an unknown one
  alone; the store write sets `context.email` only — no `updated_at` or
  `status` change, and no row re-created for an account deleted meanwhile.
- `accountLabel`: the email when present, else the name.
- A provider reporting no email leaves the row showing its name, as before.

## 8. Confidence

- **Measured:** every claim in §1, including the live `display_name=''
  context={}` state and the exact lines that drop the email.
- **Unverified, flagged:** whether every supported provider's `auth status`
  actually emits an email field. §5 is written so an absent field degrades to
  today's behaviour rather than breaking, but the per-provider matrix has not
  been enumerated and should not be assumed.
