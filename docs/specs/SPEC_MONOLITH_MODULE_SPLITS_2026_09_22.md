# SPEC — Split the two largest `agentmux-srv` files into directory modules

**Date:** 2026-09-22
**Status:** implemented in #3542 (Phase 1, `persistent.rs`) and #3544 (Phase 2, `bundle.rs`); Phase 3 is scoped in §6, not scheduled
**Author:** AgentY
**Related:**
`agentmux-srv/src/backend/blockcontroller/persistent/` (Phase 1 result),
`agentmux-srv/src/server/app_api/bundle.rs` (Phase 2 target),
`agentmux-srv/src/backend/blockcontroller/shell/` and `subprocess/` (the
directory-module precedent this follows),
`agentmux-srv/src/backend/pane_env.rs` (`SPAWN_INVENTORY`, the one
source-scanning guardrail a file move has to keep in step)

---

## 1. Why this spec exists

Two files carry a disproportionate share of `agentmux-srv`'s mass and, at
the repo's current merge rate (≈23 commits/day on `main`), of its merge
conflicts and regression surface:

| File | Lines on `main` | Shape |
|---|---|---|
| `backend/blockcontroller/persistent.rs` | 8,814 | 975 types/helpers · one 3,900-line `impl` (40 methods) · 240 trait impl · 3,660 tests |
| `server/app_api/bundle.rs` | 5,014 | 2,760 free functions (14 RPC registrations + helpers) · 2,250 tests |

Both already have a natural seam structure — the `impl` block groups into
clear concerns, `bundle.rs`'s functions cluster by RPC family, and 40–45 %
of each file is inline test modules — and both sit next to existing
directory modules (`shell/`, `subprocess/`) that show the shape the codebase
already prefers.

This spec is a **move, not a redesign.** The invariant for every phase:

> No function body changes. No type changes. No public path changes. The
> only textual edits beyond relocation are the three mechanical ones §3
> lists. Every test that existed before exists after, under the same name.

## 2. Non-goals

- Extracting the four I/O task bodies out of `spawn_process` (1,765 lines,
  the single largest function in the crate). That is a real refactor —
  the closures capture a dozen `Arc` clones each — and is scoped as Phase 3
  (§6) precisely so it is *not* mixed into a diff that reviewers should be
  able to verify as pure relocation.
- Touching the 40-odd prose comments elsewhere in the crate that say
  "`persistent.rs`". They are explanatory, not links; the module path
  `blockcontroller::persistent` they mean is unchanged.
- Rustfmt. CI does not run it, the source files were never formatted by it,
  and running it on the new files would bury the move under formatting
  noise.

## 3. The three mechanical edits, and why each is forced

A method moved from `persistent.rs` into `persistent/queue.rs` is now
declared one module level deeper. Three things follow from that and nothing
else does:

1. **Private methods become `pub(super)`.** A method with no visibility
   modifier is private to the module that *defines* it — so once
   `set_status` lives in `persistent::status`, `persistent::queue` cannot
   call it. `pub(super)` makes it visible to `persistent` and every
   descendant, which is exactly the set of callers it had before. `pub` and
   `pub(crate)` methods are left as written.
2. **Every `super::` path gains one level.** Inside the old file `super::`
   meant `blockcontroller`; inside `persistent/queue.rs` it means
   `persistent`. The rewrite is on *runs* (`(super::)+` → same run plus one
   `super::`), so the existing `super::super::obj::MetaMapType` becomes
   three levels, not four.
3. **Test modules move under `tests/` and de-indent by one level.** Each
   former `mod x_tests { … }` is now `tests/x.rs`, opening with
   `use super::super::*;` (the controller module itself), which resolves
   exactly what the inline `use super::*;` used to. De-indentation skips
   lines inside multi-line raw strings so embedded JS/JSON fixtures are
   byte-identical.

Private *fields* and private *free functions* need no change: they are
defined in `mod.rs`, and privacy in Rust is "this module and its
descendants", so every new child file sees them as before.

## 4. Phase 1 — `persistent.rs` → `persistent/` (this PR)

| File | Lines | Contents |
|---|---|---|
| `mod.rs` | 1,264 | consts, `KillRequest`, free helpers, `PersistentSpawnConfig`, `PersistentInner` + impl, the send/retry/queue enums, `PersistentSubprocessController` struct, `new`/`with_identity_stores`/`set_self_ref`, `impl Controller`, `classify_exit_line`, module map |
| `status.rs` | 132 | `set_status`, `get_status_snapshot`, `publish_status`, the heartbeat, `mark_turn_active_and_publish` |
| `eager_resume.rs` | 138 | `try_eager_resume` |
| `queue.rs` | 665 | `send_message`, `decide_send_action`, spawn-claim release, queue drain/replay, `respawn_once_for_leftover_queue`, message accept/persist |
| `resume_retry.rs` | 497 | stale-`--resume` effects, `has_prior_transcript`, `find_recovery_session_id`, `retry_after_resume_failure`, retry-batch rewrite |
| `input.rs` | 506 | `send_user_message`, `answer_question`, `deny_question`, `decide_tool_permission`, `push_stdin`, `handle_control_frame` |
| `spawn.rs` | 1,778 | `spawn_process`, verbatim |
| `lifecycle.rs` | 180 | `stop_process`, kill-request path, `session_id`, `needs_spawn`, `request_restart_when_idle` |
| `tests/` | 3,759 | nine files, one per former inline module, plus `mod.rs` |

Total 8,919 lines against 8,814 before: the delta is licence headers, file
doc comments, and the per-file `impl` wrappers.

The split was produced by a script keyed on method and module *names*, not
line numbers, so it can be re-run against a slightly different base if a
concurrent change to `persistent.rs` lands first (at the time of writing,
#3538 is open against it). The script is not committed — it is a one-shot
tool and the result, not the tool, is what gets reviewed.

### 4.1 The one non-move edit

`backend/pane_env.rs`'s `SPAWN_INVENTORY` pins every `Command::new(` site
in the crate by *file path* (retro
`docs/retro/retro-env-inheritance-instance-isolation-breach-2026-09-17.md`,
#3365 P1). One entry named `persistent.rs` for a `"echo"` that appears only
inside a comment; that comment now lives in `persistent/tests/send_input.rs`
and the entry is repointed. This is the guardrail working as designed — a
file move must not be able to slip a spawn site past it — and it is the
only test that noticed the move.

## 5. Phase 2 — `bundle.rs` → `bundle/` (next PR)

Same three edits, same script approach, keyed on top-level item names. The
planned layout:

| File | Contents |
|---|---|
| `mod.rs` | the per-agent import lock, `register`, `normalize_bundle_upsert_input`, the plain CRUD/validate/self_get registrations, `resolve_agent_for_s1`, `register_agent_project_instructions`, and the helpers every family shares (`RequirementResolution`, `resolve_account_requirements`, `bounded_display`) |
| `export.rs` | `bundle.export` |
| `components.rs` | `purge_bundle_component_refs`, `bind_imported_components`, `ResolvedComponents`/`resolve_bundle_components`, `MEMORY_NOT_EXPORTED_WARNING`, and the manifest splice helpers — used by both the per-agent export and import paths |
| `export_for_agent.rs` | `bundle.export_for_agent` and `…_with_history`, with the history size budgets |
| `import_for_agent.rs` | `bundle.import_for_agent` |
| `import.rs` | `bundle.import`, `bundle.import_preview`, `bundle.import_commit`, the ABF size/warning budgets |
| `tests/` | three files, one per former inline module |

One extra rule for free functions that Phase 1's methods did not need: items
that were already `pub(super)`/`pub(crate)` and are used from outside
`bundle` (`resolve_bundle_components`, `ResolvedComponents`,
`purge_bundle_component_refs`, `MEMORY_NOT_EXPORTED_WARNING`) become
`pub(crate)` in their new file and are re-exported from `bundle/mod.rs`
under their original path and original visibility, so
`app_api::bundle::purge_bundle_component_refs` keeps resolving for
`agent_handlers/bundle.rs`.

## 6. Phase 3 — `spawn_process` task extraction (scoped, not scheduled)

`spawn.rs` is 1,778 lines because `spawn_process` inlines four
`tokio::spawn` bodies (stdin writer, stdout reader with a `spawn_blocking`
inside it, stderr reader, process waiter) plus the argv/env build-up that
precedes them. Extracting each task body into a `pub(super) fn
<task>_task(…)` returning the `JoinHandle` would bring `spawn_process` to a
few hundred lines and make each task independently readable.

It is deliberately not in Phase 1 or 2 because it changes *what is captured
by which closure* — the only class of edit where a mechanical-relocation
review would not be sufficient, and where the race-condition history in
`persistent_resume.rs`'s module doc says the bugs actually live. It needs
its own PR with its own review standard.

## 7. Verification (Phase 1, observed)

- `cargo check -p agentmux-srv`: clean, 103 warnings — identical count to
  `main` before the change, zero new.
- `cargo test -p agentmux-srv persistent`: 149 passed, 0 failed.
- `cargo test -p agentmux-srv` (full): 4,155 passed, 0 failed, 16 ignored —
  the same 16 as `main`.
- Item-count parity between the old file and the concatenated new ones:
  212 `fn`, 84 `#[test]`, 34 `#[tokio::test]`, 13 `tokio::spawn` — all
  equal.
