# SPEC: Window-name App API hardening (phantom-id success + status codes)

**Date:** 2026-08-08
**Status:** active — §3 guards shipped same day; §4 (clear-to-default, grapheme-safe clamp, dev-discovery helper) not started. Verified 2026-08-10.
**Scope:** `agentmux-srv/src/reducer/window.rs`, `agentmux-srv/src/server/mod.rs`
**Related:** `SPEC_WINDOW_TITLE_FORMAT_2026-05-13.md` (title composition),
`SPEC_TEST_API_ACCESS.md` (dev `authkey.dev` cross-instance access),
`docs/specs/SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28` (MCP `SetName` tool)

---

## 1. Context

Agents can retitle AgentMux windows via `POST /api/v1/window/name` (the
route behind MCP `SetName(target: "window")`), including windows of *other*
instances in dev builds via the `authkey.dev` bridge. Live probing of this
surface (2026-08-08, against a running `task dev` main instance) confirmed
the core is solid — same `UpdateObjectMeta` path as the human InstancePanel
rename, persisted meta, instant reactive title update — and found two
defects plus three deferred product gaps.

## 2. Defects found (probed live, then root-caused in source)

### 2.1 Phantom window ids return `200 {"success": true}`

`POST /api/v1/window/name` with a **well-formed UUID that matches no
window** succeeds silently. Nothing changes anywhere; the caller gets a
false positive. An agent holding a stale `window_id` from an earlier
`Layout` call (window closed since) believes its rename worked.

Root cause chain:

1. `handle_window_name` → `object.UpdateObjectMeta` →
   `Command::UpdateWindowMeta` dispatched to the srv reducer
   (`server/service/object.rs:291-341`).
2. The reducer's window arm (`reducer/window.rs::handle_update_window_meta`,
   line 120) **has no existence guard** — it unconditionally emits
   `WindowMetaUpdated`. This is the *sole outlier* in its family: the
   workspace (`workspace.rs:132`), tab (`tab.rs:485`), and block
   (`block.rs:148`) meta arms all return an `Event::Error` for unknown
   ids. The comment at `object.rs:288` ("Reducer is pass-through
   (validates entity exists; emits event)") documents the intended
   contract the window arm silently violates.
3. The persist subscriber's `apply_window_meta_updated`
   (`persist_subscriber.rs:989`) then swallows the miss:
   `let Some(window) = wstore.get(...) else { return Ok(()) }`.

So: reducer doesn't validate, persistence silently no-ops, handler reports
success.

**Safety of the fix**: `persist::bootstrap_state_from_wstore`
(`persist.rs:198-237`) loads every window from wstore into
`state.windows` at startup, and `CreateWindow` inserts at runtime — the
reducer state reliably mirrors real windows, so an existence guard cannot
false-404 a live window. (A window whose `workspaceid` dangles is skipped
at bootstrap, per the comment at `persist.rs:195` — such a window becomes
un-renamable, the same exposure the tab/block guards already accept for
their own orphans.)

### 2.2 Malformed ids return `500`; not-found (post-fix) would too

`POST /api/v1/window/name` with `window_id: "no-such-window-xyz"` returns
`500 {"error": "invalid object id: ..."}` — a caller mistake reported as a
server fault. All four naming handlers (`window/name`, `tab/name`,
`pane/title`, `workspace/name`) share this via `finish_name_call`
(`server/mod.rs`), which maps **every** service error to 500. Once §2.1's
guard exists, its "window not found" error would also surface as 500
without a status mapper.

## 3. Fix (this change)

### 3.1 Existence guard in the reducer's window-meta arm

`reducer/window.rs::handle_update_window_meta` gains the same guard its
three siblings have:

```rust
if !state.windows.contains_key(&window_id) {
    let v = state.bump_version();
    return vec![Event::Error {
        code: ErrorCode::InvalidCommand,
        message: format!("UpdateWindowMeta: window not found: {}", window_id),
        fatal: false,
        version: v,
    }];
}
```

Blast radius note: `UpdateWindowMeta` is also dispatched from
`window_create.rs:304` (post-create meta stamp) — that call site runs
after `CreateWindow` inserted the window into reducer state, so the guard
is a no-op there. The frontend InstancePanel rename targets windows from
the live windows list, which exist by construction.

### 3.2 Status mapping for the naming-route family

`finish_name_call` (and `handle_window_name`'s inlined equivalent, which
this change converges onto `finish_name_call`) maps service-error text the
same way the existing `app_api_error_status` precedent does, but with
not-found split out properly:

- contains `"not found"` → **404**
- contains `"invalid"` (e.g. `invalid object id`) → **400**
- otherwise → 500 (genuine server faults)

String matching on error text is the established pattern here
(`app_api_error_status`, `server/mod.rs:852`) — the service layer returns
plain `String` errors; introducing a structured error enum across
`run_service_call` is out of scope for a hardening pass.

### 3.3 Tests

- Reducer: phantom window id → `Event::Error`, no `WindowMetaUpdated`;
  existing window → `WindowMetaUpdated` emitted (guard doesn't regress the
  happy path).
- HTTP (`server/tests.rs` router tests): `POST /api/v1/window/name` with a
  well-formed phantom UUID → **404**; malformed id → **400**; real window
  (seeded into both wstore and `srv_state`) → 200 and
  `window:displayname` persisted in wstore.

## 4. Deferred (product decisions, not defects — file separately if wanted)

1. **No clear-to-default.** Empty name is rejected (400), matching the
   InstancePanel spec's "empty → silent revert" (§2.2.2) — neither surface
   can restore the automatic title tier (workspace name / "Window N") once
   a display name is set. For agent automation (temporary status titles) an
   explicit `{"name": null}` or `DELETE /api/v1/window/name` would be
   needed. Product call; parity today is consistent.
2. **Dev-instance discovery is convention-based.** Reaching another dev
   instance requires knowing `~/.agentmux/dev/<branch>/<hash>/data/authkey.dev`
   and probing liveness yourself — stale authfiles linger after instance
   death. A `muxlog ls`-style "list live dev instances + endpoints" helper
   (or an `alive`-stamped authfile) would make cross-instance control
   ergonomic. Deliberately unfixed here: it's a new surface, not hardening.
3. **64-char clamp splits graphemes.** `chars().take(64)` can cut an
   emoji/ZWJ sequence mid-cluster. Cosmetic; fix opportunistically if the
   clamp code is ever touched again.

## 5. Verification

- `cargo test --workspace -- --test-threads=1` (CI-equivalent) green.
- Live re-probe against a dev instance after the fix ships in a dev build:
  phantom UUID → 404, malformed → 400, real window → 200 + title updates.
  (The original probe transcript that found §2.1/§2.2 is the baseline.)
