# PR Checklist: Agent Credential / Definition / Portable-Config / History Routing

**Date:** 2026-08-17
**Author:** Clamk (agent, `~/.agentmux/agents/clamk-0612a`)
**Status:** Reference checklist — distilled from
`docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`'s P1-P5 after actually
implementing steps 1-5 of that protocol (#2602, #2605, #2606, #2611, #2613). Every item below is grounded in
a real bug hit while doing that work, not a hypothetical.
**Use this when:** your PR touches anything under `identities_dir()`, `db_agent_definitions`/`db_agents`,
`db_bundles`, conversation history storage/lookup, or adds a new per-channel-isolated directory of any kind.

---

## The checklist

**1. State which scope this data is, in the PR description.** One of: CREDENTIAL, DEFINITION, PORTABLE
CONFIG (ABF), CONVERSATION HISTORY, or RUNTIME/EPHEMERAL (protocol P1's taxonomy). If you can't name it in
one word, that's a sign the change is doing two things that should be split.

**2. Route through the existing shared resolver for that scope — don't add a new `wstore`/`id_store` call
site by hand.** `identities_dir()`, `identity_history_dir()`, `agent_identity_list_for_agent()`,
`link_history_if_isolated()` etc. exist so every caller agrees. **Real incident:** PR #2587's bundle
provisioning wrote into `wstore` at all six creation call sites instead of `id_store`, making every
newly-provisioned bundle invisible outside its own channel — caught in review, not by a test, because no
test existed for "which store did this write land in."

**3. If you're adding a NEW call site for an existing shared function, don't assume upstream validation
already ran.** **Real incident:** `inject.rs`'s ordinary-spawn call into `link_history_if_isolated` passed a
raw `account_id` with no `sanitize_path_segment` check — the two pre-existing call sites happened to validate
earlier in their own function bodies, but nothing forced a third caller to. Fixed by moving the validation
*inside* the shared function itself. Prefer that shape: validate at the narrowest shared choke point, not at
every call site.

**4. Isolating credentials must not silently isolate history (or vice versa).** These are different rows in
protocol P1's taxonomy even when they live under the same directory as far as the provider CLI can tell.
**Real incident:** `identities_dir()`'s per-channel isolation (PR #2431) silently took Claude Code's
`projects/` transcripts with it — the regression this whole protocol exists to fix. If your change isolates
(or globalizes) one, check explicitly whether the other needs the opposite treatment.

**5. A new per-channel-isolated directory needs its READ path updated too, not just its write path.**
**Real incident:** PR #2431 changed where credentials+history got *written*; nothing updated
`ClaudeHistoryAdapter`'s scan list to also *read* from there, so the in-app history browser went blind to
everything written after that PR — for 10 days, on every dev/local build, before anyone noticed. If you add a
directory, grep for every place that enumerates its siblings and update the list, or explain in the PR why
you didn't need to.

**6. A provider-specific detail (a directory name, an env var, a file layout) belongs on the provider's own
config struct, not hardcoded once and assumed universal.** **Real incident:** the first cut of the
history-link fix hardcoded `"projects"` as the subdir to link — correct for Claude, silently wrong for Codex
(`sessions/`) and Gemini (`history/`). Fixed by adding `ProviderConfig::history_native_subdir` per provider.
If you're about to write a provider name as a string literal in shared code, check whether it should be a
field on `ProviderConfig` instead.

**7. Migration-phase claims ("Phase 3a", "reads still on the old table") need an executable test, not just a
doc comment.** **Real incident:** `OBJECT_SCHEMA_VERSION`'s v4 comment said reads were still on the legacy
tables; `agent_def_list()` had already flipped to the consolidated table, undocumented, and a later migration
(`m0021`) broke because it trusted the stale comment. If you change which table/store a read or write targets,
update every comment that claims otherwise, and prefer a test that asserts the actual behavior over a comment
that asserts the intended one.

**8. If a review bot (or anyone) flags something, verify it against the actual running code/binary before
changing anything — including deciding NOT to change anything.** **Real incident:** two independent review
bots claimed a Windows junction reports `is_dir()=true, is_symlink()=false` via `symlink_metadata`, which
would have meant an existing safety check was broken. It wasn't — empirically verified (twice, on the real
target OS) that junctions report `is_dir()=false, is_symlink()=true`, the opposite of the claim. No code
changed for that finding; a permanent regression test was added instead, specifically so a *future* "fix" for
the non-existent bug doesn't get made by someone who trusts the bot over the behavior. Plausible-sounding and
specific is not the same as verified — for both bot findings and your own assumptions.

**9. New filesystem code touching junctions/symlinks/reparse points must be tested empirically on the real
target OS — `cargo check` passing is not evidence it works.** **Real incident:** `ensure_history_link`'s first
version compiled and type-checked cleanly but failed at runtime (`junction::create` needs its parent
directory to pre-exist, which `create_dir_all(target_dir)` alone doesn't guarantee) — caught only because a
real filesystem test was written and actually run on Windows, not because the code looked right. If you can't
run the target platform, say so explicitly in the PR rather than asserting it works.

---

## Why this exists as its own doc, not just the protocol spec's own text

The protocol spec (`SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`) is the design record —
long, exploratory, written before implementation. This is the "what does a reviewer actually check" artifact,
written after implementation, once the taxonomy had been tested against real code and real bugs rather than
just reasoned about in the abstract. If a future PR in this area breaks one of the 9 items above, that's a
signal this checklist itself needs updating with the new incident — not that the checklist was wrong to write.
