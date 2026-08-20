# Retro: what the macOS Keychain prompt saga actually taught us

**Date:** 2026-08-20
**Area:** `agentmux-srv/src/backend/storage/muxbus.rs`, `agentmux-srv/src/identity/secret_store.rs`
**Context:** synthesizes three prior retros from this same investigation —
[retro-startup-keychain-prompt-model-catalog-2026-08-18.md](retro-startup-keychain-prompt-model-catalog-2026-08-18.md),
[retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md](retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md),
[retro-secret-store-keychain-read-timeout-2026-08-20.md](retro-secret-store-keychain-read-timeout-2026-08-20.md).
Read those for the technical detail; this one is the "what should stick"
summary, plus one genuinely new finding from field-verifying the fix on a
real affected machine.

---

## 1. The shape of the saga

Four rounds, each one uncovering a real bug the previous round didn't:

1. **model-catalog RPC** (#2654) — a purely cosmetic background feature
   (fresher model-dropdown labels) touched Keychain unconditionally on
   every app launch. Fixed by simply not doing that automatically.
2. **MuxBus 12-entry chunking** (#2665) — the actual persistent-session
   feature legitimately needs Keychain, but wrote it as 12 separate
   entries (a Windows-only size-limit workaround applied to every
   platform), so "Always Allow" on macOS never actually resolved anything
   — it just uncovered the next entry's own prompt. Fixed by writing one
   combined blob on macOS/Linux instead. Caught two real regressions in
   its own review process (a hardcoded "which layout is fresher"
   assumption that broke under the fix's own self-heal, twice).
3. **Unbounded Keychain reads** (#2679) — the fix in #2665 depends on
   successfully reading the old data to migrate it. But a Keychain read
   requiring interactive consent has no cancellation mechanism, so an
   unanswered prompt hangs forever. Fixed by bounding reads to 15s. Caught
   a real security regression (an unscrubbed plaintext copy) and a real
   correctness regression (a timed-out **write** completing later and
   silently overwriting newer state) in review — the second one meant
   *writes* stayed unbounded on purpose, only reads got the timeout.
4. **Manual field remediation** (this retro) — see §2.

Every round's reviewers (reagent, Codex) caught something real. None of
these were style nits — each was a genuine correctness or security bug in
the previous round's fix. That's the process working as intended, not a
sign anything was rushed — but four rounds for what looked like a small
fix is itself informative (§3).

## 2. The new finding: self-heal has a chicken-and-egg problem

#2665's fix is designed to be self-healing: an install with the old
12-entry layout should collapse to 1 entry automatically, the next time
its data gets read successfully.

Field-testing this on the actual machine that reported the bug: **the
self-heal never completed**, even after #2679's timeout fix. Confirmed via
a live diagnostic: the read of the old entries didn't succeed within 15s
— it just failed fast instead of hanging forever (#2679 doing exactly its
job) — so the self-heal path never got a chance to run its
migrate-and-cleanup step at all.

This is the chicken-and-egg problem: **the self-heal can only migrate data
it can successfully read, but the users who most need the fix are
precisely the ones whose reads are failing** — that's the whole reason
they're seeing the recurring prompt in the first place. A mechanism that
depends on success to fix a failure doesn't reach the users experiencing
the failure.

What actually fixed this specific machine: manually deleting the 12 stale
Keychain entries directly (`security delete-generic-password`, one call
per entry, run outside the app entirely). This sidesteps the self-heal
path completely — nothing needs to be *read* to delete it. Verified
after: a fresh read of the now-empty state returns in microseconds with
"nothing found," no prompt, because querying a *nonexistent* entry never
needs consent (only reading an *existing* one does).

This worked because a developer had shell access and knew the exact
Keychain service/account naming scheme
(`security -s agentmux -a "acct:muxbus:global:access:0"` etc., from
reading the source). **A real end user hitting this bug has no equivalent
path today.** The app itself has no "reset MuxBus connection" / "clear
stored credentials and start over" action that doesn't first require
successfully reading what's already there.

## 3. What should actually change in how we build features like this

- **A migration/self-heal path that depends on reading the broken state
  is a partial fix, not a complete one.** It correctly handles the "works
  fine, just suboptimal" case (data readable, just wastefully shaped) but
  not the "actively failing" case that motivated writing it. When the fix
  *is* the recovery path, design a variant of it that doesn't require the
  precondition that's failing — a delete-and-start-over option is often
  strictly easier to make robust than a migrate-in-place one, because
  deleting doesn't require successfully reading first.
- **This gap wasn't caught by two rounds of automated review.** Both
  reagent and Codex reviewed #2665 and #2679 closely enough to catch
  subtle ordering and security bugs in the code — but neither exercises
  the app against a real, already-broken machine, which is exactly what
  surfaced this. Automated review catches "is this code correct"; it
  can't catch "does this code's own recovery path actually reach the
  people who need it," because that requires the failing precondition to
  already exist somewhere real. There's no clean process fix for this
  short of "field-test the recovery path in the actual broken state before
  calling a self-heal complete" — worth remembering as a specific test
  step for any future self-healing/migration feature, not just this one.
- **A cosmetic feature (round 1) turned out to share a root cause with a
  real one (round 2).** Both touched the same underlying macOS Keychain
  behavior; finding the cosmetic one first didn't reveal the more serious
  one until the exact same symptom recurred on a rebuild. Worth a broader
  question for a future pass, not answered here: are there other automatic
  startup paths in this codebase touching `identity::secret_store` that
  haven't been individually audited yet? `oauth_client.rs`'s and
  `cleanup.rs`'s call sites were read during this investigation but not
  exhaustively checked against this exact failure mode.

## 4. Follow-up

- **Recommended, not yet built**: an explicit, user-facing "reset MuxBus
  connection" action (Settings or wherever account state is surfaced)
  that deletes every known Keychain entry for the account unconditionally
  — the same operation performed manually here — without first trying to
  read/migrate anything. This is the missing piece that would let a real
  user self-serve out of this exact stuck state instead of needing a
  developer with shell access. Not implemented in this pass; flagging for
  a deliberate go/no-go rather than building it unprompted.
- Same open items already carried from the prior retros: writes
  (`put`/`delete`) are still unbounded (retro-secret-store-keychain-read-
  timeout-2026-08-20.md §5), and the broader `secret_store` audit
  mentioned in §3 hasn't been done.
