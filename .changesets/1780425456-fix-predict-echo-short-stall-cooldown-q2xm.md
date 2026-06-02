---
type: patch
---

fix(term): predictive-echo — short stall cooldown so a slow rAF doesn't lock predictions out for 1.2 s

The user-visible symptom on Linux: type a sustained burst, see ~10
chars, then ~1 s of nothing, then a huge burst of all the accumulated
chars. Holds even with `--ozone-platform=x11` (PR #1241) because rAF
*occasionally* still stalls past 600 ms on broken Mutter/Chromium GPU
handoff.

The state machine in `predictive-echo.ts` had two rollback paths
sharing one cooldown:

- **`reconcile()` rollback** (line ~182): PTY echoed bytes that didn't
  match the prediction — a real divergence. Penalising with the full
  `cooldownMs` (1200 ms default) makes sense; it rides out a mode
  change without thrash.
- **`sweep()` rollback** (line ~197): no echo arrived within
  `predictTimeoutMs` (600 ms default) — *not* divergence, just a slow
  echo. Penalising this with the same 1200 ms was wrong.

The Linux symptom is exactly the second case looping:

1. User holds key, ~12 chars get predicted+painted in the first 600 ms
   (~20 Hz key-repeat × 50 ms/key = 600 ms).
2. The next rAF stalls past 600 ms (we measure occasional rAF gaps to
   ~1 s on Linux even after #1241).
3. `sweep()` fires → `rollback()` erases the painted chars +
   `enterCooldown()` sets a **1200 ms** dead zone.
4. For the next 1.2 s, every keystroke goes through `observe()`
   instead of `paint()` — nothing visible — while chars accumulate.
5. Cooldown ends → re-arms → backlog pours out at once.

This PR splits the cooldown into two:

- `cooldownMs` (divergence) — unchanged default 1200 ms.
- `stallCooldownMs` (sweep timeout) — new default **100 ms**.

Sweep now calls `enterStallCooldown(now)`. The next rAF cycle resumes
painting almost immediately. The 100 ms default is hardcoded; a
per-platform `stallCooldownMs` constructor option exists for tests.

Tests: convergence + the 17 existing predict-echo tests still pass;
added one new test specifically covering the stall path (sweep
rollback should re-paint within ~150 ms, not be stuck for ~1.2 s).

Spec: docs/specs/SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31.md
(§7.3 — cooldown split into divergence vs stall).
