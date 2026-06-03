# Gemini provider — auth loop + two latent codex-class bugs

**Date:** 2026-06-02
**Status:** diagnosed; fixes proposed (2 quick code fixes + 1 deeper auth-orchestration fix)
**Area:** agent provider auth orchestration (`cli_handlers`) + arg construction + output translation

A Gemini agent pane "doesn't do anything once loaded and never logged in." Three
distinct issues; the **auth loop is the live blocker**, the other two are the same
classes already fixed for codex in `CODEX_AGENT_LAUNCH_DOA_2026_06_02.md`.

---

## Issue 1 (LIVE BLOCKER): expired token + auth check loops without escalating to login

### Symptom
Opening the gemini pane does nothing; no browser login ever opens.

### Evidence (dev instance)
- `~/.gemini/oauth_creds.json` token is **expired ~70 days**:
  `expiry_date = 1774444778400` (≈ Mar 25) vs now `1780469278000` (Jun 3).
- The srv log shows `cli_handlers: CheckCliAuth` repeating **every ~7 s,
  indefinitely** (06:40 → 06:44+ and still going).
- **No `run_cli_login` and no `subprocess spawned` for gemini** in the session —
  only the CheckCliAuth poll. (Codex, by contrast, fired
  `run_cli_login: spawned (pipes), browser should open` and then spawned.)

### Root cause
The auth orchestration polls the provider's `authCheckCommand`
(`gemini auth status`) but, when the check keeps failing (expired/missing token),
**never escalates to the provider's `authLoginCommand` (`gemini auth login`).**
It loops on the check forever instead of triggering login. So the user is never
prompted and the agent never spawns.

### Fix (deeper — auth-launch orchestration)
In the `cli_handlers` / agent-launch auth flow: when `CheckCliAuth` returns
not-authenticated (or after N failed polls), trigger `run_cli_login` with the
provider's `authLoginCommand` instead of re-polling. Confirm whether codex's
working path special-cases something gemini misses (codex *did* escalate).
Needs a focused read of the auth-flow state machine.

### Immediate unblock (no code change)
Re-authenticate gemini interactively: `gemini auth login` (refreshes the token).
The pane then gets past the check loop.

---

## Issue 2 (latent): `--model sonnet` leak — Claude model passed to gemini

`buildRuntimeArgs` still includes `gemini` in `supportsModel`, so it appends
`--model <ModelChoice>` where `ModelChoice = opus|sonnet|haiku` are **Claude**
names. Gemini needs a gemini model (e.g. `gemini-2.5-pro`).

**Fix:** mirror the codex fix — drop `gemini` from `supportsModel` (let gemini use
its own configured default) or insert a real gemini model. `buildRuntimeArgs.ts`
line ~85. This is the sibling already flagged in the codex analysis doc.

---

## Issue 3 (latent): translator drops the turn boundary → stuck spinner

`gemini-translator.ts:34` returns `[]` for `result` (gemini's turn-end event:
`{"type":"result","status":"success","stats":{...}}`), treating it as a
no-content lifecycle event. So the conversation reducer never receives
`session_end` → never leaves the `Streaming` phase → **the working spinner never
stops** — identical to the codex bug.

Ordering is **not** affected here: gemini streams incremental `message` deltas
that the stream-parser accumulates (unlike codex's complete items), so it has no
out-of-order problem.

**Fix:** map `result` → `session_end` carrying `stats`, mirroring
`claude-translator.ts:60-66` and the codex fix. Sketch:

```ts
case "result": {
    const stats: SessionStats = {};
    const s = rawEvent.stats;
    if (s && typeof s === "object") {
        if (typeof s.input_tokens === "number") stats.input_tokens = s.input_tokens;
        if (typeof s.output_tokens === "number") stats.output_tokens = s.output_tokens;
    }
    return [{ type: "session_end", stats }];
}
```
(Remove `result` from the dropped-lifecycle `case "init":` group; keep `init` dropped.)

---

## Suggested sequencing

1. **Issues 2 + 3** — quick, mechanical, mirror the merged codex fixes; ship as a
   small `fix(gemini)` PR with `gemini-translator` + `buildRuntimeArgs` tests.
2. **Issue 1** — the auth-escalation orchestration fix; bigger, needs the
   auth-flow read. Until it lands, expired-token gemini panes require a manual
   `gemini auth login`.

## Verification
After re-auth + fixes: gemini pane launches (browser login on expired token),
runs on a gemini model (no `--model sonnet`), and the spinner stops when
`result` arrives.
