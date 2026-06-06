# SPEC: Terminal Predictive Local Echo

**Date:** 2026-05-31
**Author:** AgentA
**Status:** Draft → implementation
**Tracks:** Discussion #1161 (typing-perf umbrella) · `SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md`
**Supersedes the "shelved" call on predictive echo** (AgentY 2026-05-30) — see §2.

---

## 1. Motivation

AgentMux's terminal and VS Code's integrated terminal are **the same renderer**: xterm.js on the WebGL addon (`termwrap.ts:150`, `loadRendererAddon`; WebGL is the Windows/macOS default via `useWebGl = !term:disablewebgl`). We are **at parity on rendering**. Measured on Windows the JS path is sub-millisecond (`term-keypress` p95 0.4ms, `term-echo-render` p95 0.3ms, zero long-tasks, ~60fps).

The gap to VS Code is **structural and lives entirely in the echo round-trip**:

| | VS Code | AgentMux |
|---|---|---|
| PTY location | **in-process** (`node-pty` in the renderer/extension host) | **out-of-process** (`agentmux-srv` sidecar) |
| keystroke → echo | one in-process callback (µs) | `controllerinput` RPC **over WS** → sidecar async runtime → `portable-pty` writer (`shell.rs:761`) → PTY → echo → reader loop (`shell.rs:804`) → WPS broker → **WS** → `handleNewFileSubjectData` → base64 decode → `xterm.write` |

Every keystroke pays a cross-process, multi-hop round-trip — two WS traversals, the sidecar's async scheduler, the `#951` seq-reorder buffer, base64 — that VS Code does not. At local-PTY speeds this is small in absolute ms but it is **a meaningful fraction of a frame on top of an already-full frame budget**, and it is the only thing differentiating us from VS Code once rendering is equal. The fix is to **stop waiting for the round-trip to paint the character the user just typed.**

## 2. Why this was shelved, and why we are un-shelving it

AgentY (2026-05-30) shelved predictive echo: *"it optimizes a latency that doesn't exist locally; the ≤512B RAF-bypass already paints echo same-frame."* That reasoning conflates two things:

- **Render latency** (echo *data* → pixels): yes, already same-frame (#278 RAF-bypass).
- **Transport latency** (keystroke → echo *data arrives*): **not** zero — it is the full cross-process round-trip above, and it is exactly what the user perceives as "less immediate than VS Code."

AgentX's RFC explicitly left the door open: *"Only revisit with a sidecar-signaled tty mode + RTT telemetry."* This spec satisfies both conditions. The two real objections — **(a)** flashing plaintext over a password prompt (a security bug, not a latency one) and **(b)** corrupting CJK / TUI — are handled head-on in §6/§7, the same way VS Code's `TypeAheadAddon` handles them in production.

## 3. Goals / Non-goals

**Goals**
- Paint a just-typed printable character **in the same frame as the keydown**, before the authoritative echo returns — matching VS Code's perceived immediacy.
- **Never** render predicted text that the PTY would not have echoed (passwords, raw mode, TUI).
- **Self-correcting**: any divergence between prediction and authoritative output is reconciled to the authoritative state within ~1 round-trip, and prediction self-disables until it is safe again.
- **Zero cost** when disabled or when the round-trip is already fast (RTT-gated).
- Cross-platform; **Windows is the priority target**.

**Non-goals**
- Server-side prediction (mosh-style). Our prediction is **client-side** in `termwrap`.
- Predicting anything other than the narrow safe set in §6 (no escape sequences, no shell semantics, no autocomplete).
- Changing the authoritative data path. The PTY echo remains the single source of truth; prediction is a transient overlay reconciled away.

## 4. Invariants (the contract every change must hold)

1. **Authoritative output is never altered.** Predictions are a separate, reconcilable overlay; the xterm buffer always converges to exactly what the PTY sent.
2. **No prediction is shown unless we have positive evidence echo is on** (see §7). A password prompt must never flash plaintext.
3. **Predictions are bounded and reconciled.** A prediction that is not confirmed within `PREDICT_TIMEOUT_MS` is rolled back, and prediction disables until re-validated.
4. **Reconciliation is exact.** On any mismatch between predicted echo and actual echo, roll back **all** outstanding predictions to the authoritative buffer state, then resume from authoritative.
5. **On by default, opt-out** (`term:predictiveecho=false` disables); safety is carried by the arming gate (§7), not by being off — the RTT gate is opt-in via the threshold (§13).
6. **The keystroke frame stays sacred** (`SPEC_INPUT_RESPONSIVENESS` rule #1): prediction work is O(1) per keystroke, runs synchronously in the keydown frame, reads no layout.

## 5. Prior art (adopt, don't reinvent)

- **VS Code `TypeAheadAddon`** (`terminalTypeAheadAddon.ts`) — the canonical implementation. Maintains a queue of `IPrediction`s (character, backspace, cursor-move, tentative-boundary); writes them optimistically; matches incoming PTY output against the head of the queue; **confirms** on match, **rolls back the whole queue** on mismatch; gates on `terminal.integrated.localEchoLatencyThreshold` (default 30ms) and disables in the alt-buffer. We mirror this model.
- **mosh** — server-side speculative echo with epoch/confirmation. Not our architecture (we have no custom server protocol), but its "predict, then reconcile against authority" framing is the same.
- **xterm.js** — no built-in typeahead; predictions are implemented by writing to the terminal and tracking buffer deltas for rollback. We do the same, isolated in a `PredictiveEcho` helper.

## 6. Design — the predictive layer (frontend, `termwrap.ts`)

A new `PredictiveEcho` class owned by `TermWrap`, wired into the two existing hot paths:

- **`handleTermData(data)`** (`termwrap.ts:410`) — the send path. After `this.sendDataHandler?.(data)`, call `predict.onInput(data)`.
- **`handleNewFileSubjectData` / `doTerminalWrite`** (`termwrap.ts:442/480`) — the echo path. Before writing authoritative data, call `predict.reconcile(data)` to confirm/roll back predictions, then write.

### 6.1 What is predicted (the safe set)

Per keystroke `data` (the string xterm's `onData` produced):

| Input | Prediction |
|---|---|
| single printable, single-width ASCII/Latin (`0x20–0x7E` and confirmed-safe Unicode) | write the glyph, advance the predicted cursor by 1 |
| `\r` / `\n` (Enter) | **tentative line boundary** — predict cursor to col 0 of next row *only* if we have a clean prediction state; otherwise flush predictions and stop (the shell may print a prompt) |
| `0x7F` / `\b` (Backspace) | erase the previous predicted cell, retreat cursor by 1 — **only if** the last outstanding item is our own predicted glyph (never erase authoritative cells) |
| anything else (arrows, Ctrl-*, ESC, paste, multi-char) | **flush + do not predict** this keystroke |

Everything outside this table is left entirely to the authoritative round-trip. CJK / multi-byte / wide chars are **not** predicted (width ambiguity) until §9-phase-3 adds width-aware prediction; until then they fall through to authoritative echo with no regression.

### 6.2 Prediction queue + cursor model

```
interface Prediction {
  kind: "char" | "backspace" | "newline";
  cell?: { row: number; col: number };   // buffer coords where we wrote
  glyph?: string;                          // the predicted character
  expected: string;                        // the exact bytes we expect the PTY to echo for confirmation
  at: number;                              // performance.now() when predicted (for timeout + RTT)
}
```

`PredictiveEcho` keeps `queue: Prediction[]` and a `predictedCursor` (row,col) tracked from xterm's authoritative cursor at the moment the queue was empty, advanced locally per prediction. We never query xterm layout on the keydown path; we read `terminal.buffer.active.cursorX/Y` only when the queue transitions empty→non-empty (cold start of a burst).

### 6.3 Reconciliation algorithm (`reconcile(chunk)`)

On each authoritative `chunk` from the PTY, before writing it:

1. If `queue` is empty → write `chunk` normally, done.
2. Walk `chunk` against the head of `queue`:
   - If the chunk's next bytes **equal** `head.expected` → **confirm**: pop `head`, consume those bytes from `chunk` (the cell is already painted correctly; nothing to redraw), continue.
   - If they **diverge** → **rollback**: discard the remaining `queue`, restore the affected cells to their pre-prediction authoritative content (re-issue an `xterm.write` of the authoritative bytes over the predicted region using saved cell snapshots, or the simpler/robust path — see §11), then write the *entire* original `chunk` authoritatively from the rollback point. Enter `cooldown` (§7.3).
3. Any `queue` entries older than `PREDICT_TIMEOUT_MS` with no matching output → treat as divergence (rollback + cooldown). This is the password/echo-off catch: predictions that are never echoed time out and vanish. The sweep is driven by **both** the PTY-chunk path *and* the keystroke stream (`onInput` sweeps before handling each key) — **no wall-clock timer** (project rule: no grace timers). So when echo stalls and no chunk arrives, the next key the user presses is what ages out a stale prediction.

Confirmation is **byte-exact** against `expected`, so terminals that echo differently than we predicted (e.g. `^C`, tab expansion, autosuggest) are caught as divergence and reconciled, never left wrong.

## 7. tty-mode safety (the gate that makes §4.2 hold)

Two layers, defense-in-depth:

### 7.1 Observational gate (all platforms, always on)

Prediction is only **armed** after we have **recently observed our own input echoed back** (a confirmed `char` prediction within the last `ECHO_CONFIRM_WINDOW_MS`). Concretely:
- Start in `unarmed`. The first keystroke of a session/line is **not** predicted; it is sent and we watch whether the PTY echoes it.
- On a byte-exact echo confirmation → `armed`; subsequent keystrokes in the burst predict.
- **Disarm at every boundary.** Any **non-printable input** (Enter, arrows, Ctrl-\*, Esc — i.e. the keys that launch `sudo`/`ssh`/`vim` or move into a new mode) flushes *and* clears `armed`. This is the critical fix for the armed→echo-off transition: because a password prompt or TUI is always reached *through* a non-printable key (the Enter that runs the command), the first keystroke of the new context starts `unarmed` and is observed, not painted. Without this, an already-armed session would paint the first password char as plaintext.
- This means at an **echo-off boundary** (entering a password prompt, `vim`, raw mode) the very next keystroke is sent **without** a prediction — there is no plaintext to flash, because we stopped predicting the moment confirmations stopped.

This is the same self-correcting principle VS Code relies on, hardened: we require *positive* confirmation to predict rather than predicting-until-proven-wrong, trading one round-trip of "first char of a burst is authoritative-only" for **zero plaintext flash**.

### 7.2 Explicit sidecar tty-mode signal (zero-flash, Unix first)

To remove even the within-burst edge (an app turns echo off mid-line), the sidecar signals echo state:

- **Unix (Linux/macOS):** `shell.rs` holds the `portable-pty` master. Read the line-discipline flags via `tcgetattr` on the master fd (`ECHO`, `ICANON`); poll on a cheap interval and on known mode-changing events, emit a `term:mode` event `{ echo: bool, canonical: bool, altscreen: bool }` over the existing WPS broker alongside the data stream. The frontend disarms prediction immediately when `echo=false || altscreen=true`.
- **Windows (ConPTY) — the priority platform:** ConPTY does **not** expose termios. We therefore rely on (a) the §7.1 observational gate (primary), plus (b) **alt-screen detection by parsing the output stream** for `CSI ?1049h` / `?47h` / `?1047h` (enter alt buffer ⇒ TUI ⇒ disarm) and `?1049l` (leave ⇒ re-evaluate). Alt-screen parsing is done where the chunk is already being scanned in `reconcile`, so it is free. Password prompts on Windows are covered by §7.1 (no confirmed echo ⇒ unarmed ⇒ no prediction). This matches VS Code's own Windows behavior.

### 7.3 Cooldown

Two distinct cooldowns prevent different failure modes:

- **Divergence cooldown** (`COOLDOWN_MS`, default 1200 ms): after a `reconcile()` rollback (PTY bytes didn't match the prediction) or explicit `echo=false`. Re-arms only on a fresh observed echo confirmation. Rides out mode changes without flapping.
- **Stall cooldown** (`STALL_COOLDOWN_MS`, default 100 ms): after a `sweep()` timeout (no echo arrived within `PREDICT_TIMEOUT_MS`). **Not** a divergence — just a slow echo (e.g. an rAF stall on Linux). Re-arms on the next rAF cycle so predictions resume within ~100 ms instead of being locked out for the full `COOLDOWN_MS`.

Before this split, sweep timeouts used the divergence cooldown, causing a visible "10 chars → ~1 s pause → burst" pattern on Linux when rAF stalled past `PREDICT_TIMEOUT_MS`.

## 8. RTT telemetry + latency gate

`PredictiveEcho` measures round-trip continuously: `confirm.at - prediction.at` per confirmed char → rolling p50 RTT. Prediction is only active when **rolling p50 RTT > `PREDICT_THRESHOLD_MS`** (default 12ms ≈ ¾ frame). When the local round-trip is already sub-threshold (nothing to hide), prediction stays dormant — zero risk, zero benefit foregone. This is AgentX's "RTT telemetry" condition and VS Code's `localEchoLatencyThreshold` analog. The same RTT samples feed `bench-term-echo` parity reporting.

## 9. Phased plan

- **Phase 0 — instrument the round-trip (no behavior change).** Add a `term-roundtrip` measure (keystroke-sent → first matching echo byte) to `termwrap`, surfaced in the Perf HUD + `bench-term-echo`. Fix the bench's stale-`authkey.dev` discovery (prune dead-pid entries) so we have a clean baseline number. **Gate**: a real Windows p50/p95 round-trip figure recorded in this spec.
- **Phase 1 — core predictive echo (cooked mode, single-line, ASCII).** `PredictiveEcho` class; §6.1 char/backspace/newline; §6.3 reconcile; §7.1 observational gate **+ disarm-at-every-boundary** (non-printable input *and* a basic alt-screen-**enter** parse `CSI ?1049h|?47h|?1047h` ⇒ disarm, folded into `reconcile` since it already scans the chunk); §7.3 cooldown; §8 RTT gate; keystroke-driven sweep (§6.3). Setting `term:predictiveecho` default **on** (opt-out), threshold default **0** (always predict once armed). **Gate**: byte-exact reconciliation property test; manual password-prompt test shows **zero** plaintext flash; `vim`/alt-screen shows no corruption.
- **Phase 2 — explicit Unix tty-mode signal (§7.2a)** for zero-flash within-line, + the **full** alt-screen parser (§7.2b: enter/leave/re-evaluate) for all platforms (Phase 1 ships only the enter⇒disarm half).
- **Phase 3 — width-aware prediction**: CJK/wide-char + line-wrap + backspace-across-wrap correctness.
- **Phase 4 — tune + default-on**: threshold tuning, optional dim styling for unconfirmed cells (VS Code dims; we evaluate), flip default after a soak. RTT-gated so default-on is safe where it does nothing.

## 10. Edge cases (explicit handling)

- **Paste / bracketed paste / multi-char `onData`** → flush, no prediction (§6.1).
- **Fast typing (multiple pending predictions)** → queue handles N outstanding; reconcile consumes them in order; bounded by `MAX_QUEUE` (flush+disarm beyond it).
- **`#951` seq-reorder** → unchanged; prediction is purely frontend and keys off the *echo* stream order, which the seq-reorder already normalizes.
- **Resize** → flush queue (cursor model invalid until next confirmed cold-start).
- **Scroll / output arriving while typing (e.g. a background log)** → divergence on the first non-matching byte ⇒ rollback ⇒ authoritative wins. Safe by construction.
- **Backspace at column 0 / across a wrapped line** → not predicted until Phase 3; falls through to authoritative.
- **Disconnect / PTY death** → flush, disarm.

## 11. Rendering & rollback in xterm.js (the hard part, called out)

xterm.js has no "tentative cell" API. Two implementation options, decided in Phase 1:

- **(A) Write-and-rewrite (simplest, robust):** predictions are normal `terminal.write`s. On rollback, snapshot the affected cells *before* predicting (cheap: a small ring of `{char, attr}` per predicted cell from `buffer.active.getLine().getCell()`), and on divergence re-write the saved authoritative content, then the real chunk. Risk: a one-frame visible correction on the rare mismatch — acceptable (mismatches are rare and the corrected frame is authoritative).
- **(B) VS Code-style buffer decorations** (overlay rendering of predicted glyphs without committing to the buffer): zero-rewrite rollback, but couples to xterm internals.

Start with **(A)**; it satisfies all invariants and isolates risk. Re-evaluate (B) only if (A)'s rollback frame is perceptible.

## 12. Testing / verification

- **Property test** (`PredictiveEcho.reconcile`): for random interleavings of (printable input, matching echo, diverging echo, echo-off) the buffer **always** converges to the authoritative byte stream and never shows an unconfirmed-then-unmatched glyph beyond `PREDICT_TIMEOUT_MS`.
- **Safety manual matrix:** (1) `read -s` / `sudo` password prompt → **no** plaintext flash; (2) `vim` insert mode → no corruption, prediction disarmed in alt-screen; (3) CJK input → unchanged (no prediction, no regression); (4) fast hold-key → no artifacts.
- **Latency:** `bench-term-echo` round-trip before/after must be unchanged (authoritative path untouched); the **perceived** keystroke→paint, measured with the Phase-0 `term-roundtrip` minus prediction (i.e. paint happens in the keydown frame) → ~0 perceived for predicted chars.
- **Regression:** `bench-agent-keystroke` / the CI input-latency guardrails (#1148/#1174) stay green; keydown path adds no layout read.

## 13. Config, default, rollout

- `term:predictiveecho` (bool, default **true** — set `false` to opt out) — master enable.
- `term:predictiveecho:thresholdms` (number, default **0** = always predict once armed; set >0 to re-enable the §8 RTT gate).
- `term:predictiveecho:dim` (bool, default false) — optional unconfirmed-cell styling.
- Shipped **default-on**: the observational arming gate (§7.1) — never predict without a confirmed echo — carries the safety (no password flash), and any divergence self-reconciles, so the blast radius is bounded. Re-enable RTT-gating per-platform via the threshold if a platform proves prediction unnecessary there.

## 14. Cross-platform notes

- **Windows (priority):** observational gate (§7.1) + alt-screen parse (§7.2b). No termios; this matches VS Code's Windows posture and is sufficient for the password/TUI safety invariants.
- **Linux/macOS:** add the explicit `tcgetattr` echo signal (§7.2a) for zero-flash within-line. Note Linux also defaults to the **DOM** xterm renderer (`termwrap.ts:549`) — orthogonal to this spec but the larger Linux smoothness lever; tracked separately.
- The authoritative data path, `#951` seq-reorder, and the WPS broker contract are **unchanged** on every platform.

## 15. Risks

| Risk | Mitigation |
|---|---|
| Plaintext flash over password | §7.1 positive-confirmation arming (no predict without observed echo) + §7.2 explicit/alt-screen disarm + invariant #2 as a merge gate |
| TUI/CJK corruption | not in the safe set (§6.1); alt-screen disarm (§7.2b); width-aware deferred to Phase 3 |
| Rollback frame visible | rare (byte-exact confirm); option (A) accepted, (B) escape hatch |
| Keystroke-frame regression | O(1) per key, no layout read; CI guardrails (#1148/#1174) |
| Flapping in ambiguous modes | cooldown (§7.3) + RTT gate (§8) |

---

**Next:** Phase 0 (round-trip instrument + bench fix) → Phase 1 (core, default-off) behind `term:predictiveecho`, each a focused PR with the property test + the safety manual matrix as merge gates. Append findings/decisions to discussion #1161.
