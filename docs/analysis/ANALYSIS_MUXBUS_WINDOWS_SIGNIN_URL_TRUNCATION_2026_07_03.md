# ANALYSIS: MuxBus Cloud sign-in fails on Windows — OAuth URL truncated by `cmd /C start`

**Date:** 2026-07-03
**Status:** **RESOLVED — shipped in #1938** (`fix(muxbus): quote Windows
browser-open URL so cmd.exe doesn't truncate it at the first &`). Verified
2026-08-29 (docs-cleanup Phase 4): `agentmux-srv/src/util.rs`'s
`open_browser` now builds `start "" "{url}"` via `raw_arg`, with a comment
recording exactly why `Command::arg` is wrong here (MSVCRT quote escaping
vs. `cmd.exe`'s quote-toggle parser). Originally this line cited the
in-flight branch `agent3/muxbus-windows-open-browser` rather than a merged
PR — accurate when written, but it left the doc reading as unshipped work
long after it landed.
**Scope:** `agentmux-srv/src/muxbus/pkce.rs`, `agentmux-srv/src/identity/oauth_client.rs`
**Trigger:** Clicking "Connect with AgentMux" (Armory → Accounts, the per-agent identity
panel, or the statusbar HostPopover — all three share `AgentMuxConnectPanel.tsx`'s
`useMuxBusStatus().connect()`) opens a browser tab that lands on
`https://auth.muxbus.agentmux.ai/error?error=Required+parameters+missing` instead of
the Cognito hosted-UI login page.

---

## 0. TL;DR

**Bug:** On Windows, `open_browser()` shells out via `cmd /C start "" <url>` with the
URL passed as a bare (unquoted) argument. `cmd.exe`'s `/C` mode re-parses its entire
trailing argument as a batch command line, where `&` is an unescaped command
separator. An OAuth authorize URL is built entirely from `&`-joined query params
(`?response_type=code&client_id=...&redirect_uri=...&scope=...`), so `cmd.exe` splits
it at the *first* `&` and only ever launches the browser with
`.../oauth2/authorize?response_type=code` — every parameter after that (`client_id`,
`redirect_uri`, `scope`, PKCE challenge, `state`) is silently dropped.

Cognito receives a request with no `client_id`, which is indistinguishable (from its
error handling) from a request with an empty `client_id`, and returns its generic
`error=Required+parameters+missing` page. This reproduces the user's exact symptom
byte-for-byte (confirmed by hitting the live endpoint directly — see §2).

**Blast radius:** every Windows install, both browser-based OAuth flows in the repo:
- `muxbus/pkce.rs` — AgentMux Cloud sign-in (MuxBus PKCE).
- `identity/oauth_client.rs` — the generic Armory service-OAuth flow (Google/Microsoft
  code-flow providers), currently gated behind unprovisioned client IDs
  (`client_id: None` — see the module's own "Scaffold status" doc-comment) but would
  hit the identical failure the moment a provider is activated.

This is **not** a deploy-timing issue. The branded `auth.muxbus.agentmux.ai` Cognito
domain (agentmux-cloud PR #21) is live and correctly configured — confirmed directly
against the endpoint (§2). The desktop-side flip to that domain (agentmux PR #1882,
shipped in v0.49.12) is unrelated to this bug; the same truncation would happen
against the legacy `muxbus-auth.auth.us-east-1.amazoncognito.com` prefix domain too,
since the bug is in how the URL reaches the browser, not which domain it points at.

**Fix:** quote the URL as a single `raw_arg` token so `cmd.exe` stays in quoted mode
for the whole string (§3). Deduplicated the two copies of `open_browser()` into
`agentmux-srv/src/util.rs`.

---

## 1. Where the two flows call the vulnerable code

`frontend/app/view/accounts/AgentMuxConnectPanel.tsx` is the single shared
implementation for AgentMux Cloud connect, rendered from three places:
- `MuxBusConnectSection` — per-agent identity panel (`AgentIdentityPanel.tsx`)
- `AgentMuxConnectPanel` — Armory → Accounts gallery tile (`accounts-manager.tsx`)
- `frontend/app/statusbar/HostPopover.tsx` — the statusbar popover ("status panel")

All three call the same `useMuxBusStatus().connect()`, which RPCs `muxbus.login` →
`agentmux-srv/src/server/muxbus_handlers.rs` → `crate::muxbus::pkce::run_pkce_login()`
→ `open_browser(&auth_url)` (`pkce.rs:85`, prior to this fix). The frontend's
`isConfigured()` gate only checks that a build-time `client_id` string is non-empty —
it has no visibility into what actually reaches the browser, so it cannot catch this
class of bug.

`agentmux-srv/src/identity/oauth_client.rs` had a byte-for-byte identical
`open_browser()` (same `cmd /C start "" <url>` pattern) for the generic Armory
service-OAuth flow.

---

## 2. Confirming the exact failure against the live endpoint

Hitting `https://auth.muxbus.agentmux.ai/oauth2/authorize` directly (this machine,
2026-07-03) shows the domain is live and genuinely Cognito (real
`x-amz-cognito-request-id` response headers, not a CloudFront placeholder):

| Request | Response `Location` |
|---|---|
| Full valid query string, garbage `client_id=test` | `.../error?error=invalid_request&client_id=test` |
| `client_id` param entirely absent | `.../error?error=Required+parameters+missing` (no `client_id` echoed) |
| `client_id=` (present, empty) | `.../error?error=Required+parameters+missing` (no `client_id` echoed) |
| Only `?response_type=code` (everything after the first `&` dropped) | `.../error?error=Required+parameters+missing` |

The last row is the exact URL the user saw, and it's exactly what `cmd.exe` produces
when handed the real `auth_url` unquoted — reproduced directly against `cmd.exe` on
this machine:

```
$ cmd.exe //C "echo a&echo b&echo c"
a
b
c
```

i.e. `cmd /C "<string>"` treats each unescaped `&` as a command separator, executing
`echo a`, `echo b`, `echo c` as three independent commands. Applied to
`start "" https://.../authorize?response_type=code&client_id=...&...`, only
`start "" https://.../authorize?response_type=code` ever runs; `client_id=...`,
`redirect_uri=...`, etc. get interpreted as (failing) commands of their own and never
reach the browser.

Quoting suppresses this:

```
$ cmd.exe //C "echo \"https://x/?a=1&b=2\""
\"https://x/?a=1&b=2\"
```

The whole string survives as one token when wrapped in quotes cmd.exe itself
recognizes (confirmed empirically before writing the fix, and locked in as an
automated test — see §3).

---

## 3. The fix

`agentmux-srv/src/util.rs` (new — previously duplicated verbatim in `pkce.rs` and
`oauth_client.rs`):

```rust
pub fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .arg("/C")
            .raw_arg(format!("start \"\" \"{url}\""))
            .spawn();
    }
    // macos / other unchanged
}
```

`raw_arg` (not `arg`) is required: `Command::arg` would apply the MSVCRT-style
quote-escaping Rust normally uses for Windows argv, which `cmd.exe`'s own (much
simpler, escaping-unaware) quote-toggle parser does not understand — a naively
`format!("\"{url}\"")`-wrapped value passed through `.arg()` gets re-escaped by Rust
and would break out of the quoted region again. `raw_arg` hands `cmd.exe` the exact
bytes it should parse, matching its actual quoting model.

Two regression tests added in `util.rs` (`cfg(all(test, target_os = "windows"))`, run
by the existing `windows-latest` CI job):
- `unquoted_url_is_truncated_by_cmd_ampersand_splitting` — pins the *old*
  broken behavior so the bug can't silently reappear unnoticed in a refactor.
- `quoted_url_survives_cmd_ampersand_splitting` — proves the fix: the same
  `raw_arg` construction used by `open_browser`, run through real `cmd.exe`, and
  asserts the full URL (with embedded `&`) survives as one token.

Both pass locally against real `cmd.exe` on this machine, and existing `muxbus::*`
and `identity::oauth_client::*` test suites remain green (11/11).

---

## 4. Non-goals / out of scope

- Did not touch the agentmux-cloud Cognito/CDK side — it's confirmed working correctly
  (§2) and PR #21's deploy notes were followed correctly by whoever ran `cdk deploy`.
- Did not change macOS/Linux `open_browser` — `open` (macOS) and `xdg-open` (Linux) are
  passed the URL as a single argv element via `Command::arg`, which does not go through
  a secondary shell re-parse, so they were never affected.
- Left `identity/oauth_client.rs`'s per-provider OAuth flow otherwise untouched — it's
  still scaffold-gated (`client_id: None`) and unreachable from the UI today; this fix
  just ensures it doesn't inherit the same bug the moment a provider is activated.
