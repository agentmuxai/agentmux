# AgentMux Repo Health Audit II — Dead Code, Duplication, Legacy Remnants

**Date:** 2026-07-20
**Baseline:** `agentmux` main @ latest (v0.54.2, post `chore: release v0.54.2`)
**Method:** compiler-warning ground truth (`cargo build --workspace`, 153 warnings captured) triaged item-by-item against source, plus 3 parallel deep-research passes (Rust duplication across every `agentmux-srv`/`agentmux-cef` handler file and every documented Rust↔TypeScript "keep in sync" pair; frontend dead/duplicate code; legacy/stale-subsystem remnants), each independently fanning out further sub-agents where the surface area warranted it. Read-only — no changes made in this pass.
**Relationship to the prior audit:** this is a direct sequel to `docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md` (15 days prior). Several items that report flagged are **confirmed still unfixed** today (noted inline below); this report does not re-derive that report's architecture/tree-shake/docs/cross-repo sections — read both together.

---

## 0. Executive summary

The dominant finding this pass is different in kind from the 07-05 audit: alongside the expected dead-code and duplication inventory, cross-checking every documented Rust↔TypeScript "mirror" pair surfaced **five real, live correctness bugs** — not hygiene issues — that had been sitting undetected because nothing enforces the "keep in sync" comments most of these pairs carry. Two of the five are silent data-loss/data-swallowing bugs; none are covered by any test. See §1 — read this section first, it's the part worth acting on soonest.

Beyond that:

- **~30 genuinely dead Rust items** (functions/fields/variants with zero callers anywhere, not just this build) are safe to delete today, cross-verified against the 07-05 audit where they overlap (several, e.g. the `wndproc.rs` frameless cluster and `resolve_provider_alias`, were already flagged 15 days ago and are still present).
- **~20 UNCLEAR Rust items** are not simple dead code — they're abandoned-mid-flight scaffolding (a `Bootstrapping` lifecycle phase never wired, a `container.rs` stop/remove pair never hooked into any delete flow, an `AccountNotFound` variant nothing produces) that need a roadmap decision, not a deletion.
- **~35 real Rust-side duplication findings** across RPC handlers, most small (2-10 line blocks) and low-risk to consolidate; a handful are large near-duplicate function pairs (`compute_and_ensure_{bundle,account}_dir`, the OAuth pipe-vs-PTY success/failure match blocks) that are explicitly self-documented as intentional copies, not accidental drift.
- **Frontend: one dead directory (12 files) plus 3 dead components/exports**, ~10 duplicate-utility findings, 2 duplicate SCSS keyframe clusters, and confirmation that a shared `useWaveEventSubscription` hook would collapse ~20-25 copy-pasted subscribe/cleanup call sites.
- **Legacy remnants:** a dead Go subproject still wired into `Taskfile.yml` (11 tasks), several Tauri-era scripts/comments still present 15+ days after the 07-05 audit flagged the pattern, and — the most externally-visible miss — the `preset.*` naming survives as the *primary* (non-aliased) identifier on the MCP tool surface (`PresetList`/`PresetGet`), the one place in the whole preset→bundle rename where the old name never became a deprecated alias.

---

## 1. Real bugs surfaced by the audit (read this first)

These were found while auditing Rust↔TypeScript "mirror" pairs and Rust-side migrations for duplication/drift — they are not cleanup items, they are live correctness gaps. None are covered by a test that would catch them.

### 1.1 Rust's Claude Code translator silently reports API failures as success
`agentmux-srv/src/agents/translator/claude.rs` claims to mirror `frontend/app/view/agent/providers/claude-translator.ts` (both header comments cross-reference each other). Three real gaps:

- **`handle_result` (`claude.rs:248-266`) never reads `is_error`/`api_error_status`** on the `result` frame — it unconditionally emits `Cost` + `Done` as if the run succeeded. TS's equivalent (`claude-translator.ts:78-86`, tested `claude-translator.test.ts:274-326`) emits an `error_result` event whenever `rawEvent.is_error === true`. `AgentEvent::Error { message }` already exists (`agentmux-srv/src/agents/types.rs:107-110`) but `claude.rs` never constructs it. **Consequence:** an authentication failure, a rate-limit-to-death, or a network error during a drone-orchestrated agent run is reported to the drone block as a normal successful completion, not a failure.
- **`rate_limit_event` frames are silently dropped** — `translate()`'s match (`claude.rs:92-98`) has no arm for it; falls to the wildcard `_ => {}`. TS translates this into `provider_waiting` (`claude-translator.ts:61-69`, tested).
- **`message_start` frames are unhandled** — TS has an explicit, tested fallback (`handleMessageStart`, `claude-translator.ts:278-288`) to extract `tool_result` blocks when a `message_start` frame has `role === "user"`, a documented real wire shape. Rust's `handle_stream_event` (`claude.rs:110-175`) only switches on `content_block_{start,delta,stop}`; `message_start` hits the wildcard with a bare discard comment.

**Recommendation:** file as a bug, not a cleanup item. The drone/orchestration path (whatever consumes `agentmux-srv`'s `AgentEvent` stream, distinct from the live-render path this session's own work never touched) is currently blind to Claude API failures and rate-limiting for any agent it drives.

### 1.2 `agents_consolidate.rs` migration silently drops `working_directory` data
`agentmux-srv/src/backend/storage/agents_consolidate.rs`: `DefRow.working_directory` is read from the legacy-table `SELECT` (line 115) but the subsequent `INSERT OR REPLACE INTO db_agents` (lines 149-193) hardcodes `working_directory` as `''` instead of using `def.working_directory`. This is the migration that consolidates legacy agent definitions into `db_agents` — every legacy agent's configured working directory is silently discarded during consolidation. Flagged by the dead-code triage as `field working_directory is never read` (only true because nothing reads the struct field it's assigned to before the bug discards it) — the warning is a false-positive-shaped symptom of a real bug underneath.

**Recommendation:** fix the `INSERT` to use `def.working_directory`; this is very likely a real, live data-loss bug for any install that still has legacy-table agent definitions to consolidate.

### 1.3 `AGENT_SLUG` / `WORKING_DIR` template variables missing from the production Rust config-builder
`agentmux-srv/src/backend/agent_config.rs`'s `build_config_files` (the function actually called in production — sole caller `agentmux-srv/src/server/app_api/agent_open.rs:575`) sets template variables `AGENT`, `AGENT_DISPLAY`, `AGENT_ID`, `DATE` — but never `AGENT_SLUG` or `WORKING_DIR`. `frontend/app/view/agent/agent-model.ts`'s `buildConfigFiles` (:711) sets both. **Consequence:** any CLAUDE.md/soul/skill content authored with `{{AGENT_SLUG}}` or `{{WORKING_DIR}}` renders correctly when built by the TS path but leaves the literal placeholder text un-substituted when built by the Rust path — a visible, silent content bug for any agent launched through the backend's own config-gen rather than the frontend's.

Separately, `build_config_files_with_bus` (`agent_config.rs:153-239`, confirmed zero callers anywhere — dead code, see §2) does set `WORKING_DIR` but is unreachable; if anyone ever wires it up expecting it to be the "more complete" variant, it would also silently regress `.claude/settings.json` generation — it writes raw `content_map["hooks"]` directly to the deprecated `.claude/hooks.json` path instead of merging through `build_settings_with_hooks`, the exact anti-pattern that caused the documented v0.33.804 streaming bug (hooks written where Claude Code never reads them, silently disabling the PreToolUse hook).

**Recommendation:** add `AGENT_SLUG`/`WORKING_DIR` to `build_config_files`'s template vars; delete `build_config_files_with_bus` rather than fix it (nothing calls it, and fixing it means re-deriving `build_settings_with_hooks`'s logic a second time).

### 1.4 Editor and Toolchain panes bypass the CEF-safe clipboard wrapper
`frontend/app/view/editor/editor-view.tsx:668-674` and `frontend/app/view/toolchain/toolchain-view.tsx:246` both call raw `navigator.clipboard?.writeText(...)` directly. `frontend/util/clipboard.ts`'s own header comment states CEF blocks unprivileged clipboard access and mandates routing through the host IPC wrapper — which every other clipboard-writing call site in the codebase correctly does (e.g. `PreLaunchAuthPanel.tsx:279/310`, `OAuthConnectPanel.tsx:525`). **Consequence:** "Copy" in the Editor and Toolchain panes may silently no-op under CEF's permission policy.

**Recommendation:** switch both call sites to the shared wrapper. Cheap, two-line fix per site.

### 1.5 Identity `assigned_agents` field is split-brain: writer says dead, two live readers still depend on it
`frontend/app/view/identity/identity-model.ts:66-73` marks `assigned_agents: string[]` `@deprecated ... do not write new code that reads this field`, and `backendToAccount()` (:267-278) unconditionally sets it to `[]` for every backend-loaded account. But `identity-view.tsx`'s `AssignmentsTab` (:378, :384) and the account edit form (:490, :571-583) still read/write it live. **Consequence:** the Assignments matrix UI can never show real assignment data for any backend-loaded account — it's permanently empty for the deprecated field's designated readers, a genuine (if quiet) UI regression baked in by the deprecation itself.

**Recommendation:** this needs a design decision (finish migrating `AssignmentsTab`/the edit form to the replacement — "derive from the agent-side reverse index" per the deprecation comment — or un-deprecate and keep populating it) rather than a mechanical fix; flagging for the identity-surface owner.

---

## 2. Dead Rust code (compiler-warning-verified)

`cargo build --workspace` on this platform (Windows) surfaces 153 warnings; every `never used`/`never read`/`never constructed` one was triaged individually against source (not just the warning text) to separate genuine dead code from `#[cfg(target_os)]`/test-only false positives. Full 77-item table available in the audit's working notes; summarized by verdict below.

### 2.1 Genuinely dead — safe to delete now (~22 items)

Grouped by why they're dead, not just where:

**Superseded by a `_unique`/newer variant, old one never called:**
- `mcp_server_upsert` (`storage/mcp_servers.rs:149`) — superseded by `mcp_server_upsert_unique`/`_unique_global`, the only variants any caller uses.
- `skill_upsert` (`storage/skills.rs:347`) — same pattern.
- `get_stdin_tx` (`shell_node.rs:108`) — superseded by `resolve_stdin`.
- `send_message` (`blockcontroller/acp.rs:169`) — superseded by the `BlockController::send_input` trait impl, which builds an equivalent request inline instead.

**Zero callers anywhere, doc claims a caller that doesn't exist:**
- `is_armed` (`teardown_backstop.rs:83`) — sibling `arm`/`disarm`/`should_teardown` all have real wrapper callers; this one has none.
- `pending_count` (`browser_pane/auth.rs:163`) — doc says "useful for leak checks in tests" but the file has no test module.
- `peek_back_pending_window_creation`, `has_browser`, `list_browser_labels` (`state.rs:1388`) — doc claims a caller in `wrr/win_event.rs` that grep doesn't confirm; functionally-equivalent `get_browser`/`list_browsers` (post-H.2-migration) are what's actually used everywhere.
- `resolve_provider_alias` (`providers.rs:425`) — **still present 15 days after the 07-05 audit flagged it.**
- `send_to_conn` (`eventbus.rs:150`) — real call sites use `send_to_conn_lane` directly.
- `GitHubContext` struct (`storage/agents.rs:196`) — never constructed/serialized/parsed; `github_context` is carried everywhere as an opaque string/JSON blob.
- `into_bundle_id` field (`identity/auth_session.rs:80`) — `finish_success` takes an explicit fresh parameter instead of reading it.
- `UNKNOWN` const (`identity/resolver.rs:50`) — the literal `"unknown"` is hardcoded directly at `resolver.rs:932` instead of referencing it; the const's whole reason for existing isn't honored anywhere it's used.
- `ps` field (`osc_extractor.rs:34`), `bundled` field (`tool_store.rs:38`), `msg_type` field (`messaging/whatsapp/types.rs:48`) — each constructed but genuinely never consulted by any logic path (unlike sibling fields in the same structs, which are read repeatedly).
- `ipc_port` field (`cef/client/mod.rs:152`) — vestigial; every real read goes through `resolved_ipc_port()` instead (a documented fix for a floating-pane bug where `ipc_port=0` at construction).

**Retired-by-design, self-documented no-ops kept only for migration-id stability:**
- `backfill_direct_links` (`migrations/m0013_agent_direct_bindings.rs:50`), `backfill_latest_instance_only` (`migrations/m0014_agent_direct_bindings_rerun.rs:47`) — own comments confirm the bodies are intentional no-ops (source tables dropped in Phase 4c); safe to leave inert but genuinely have zero live callers if ever removed alongside their migration-id retirement window.

**Windows-build-only dead (real dead code on this platform, not a cfg false positive):**
- `setup_native_frameless`, `install_frameless_resize_hook` (`client/wndproc.rs:17,64`) — `#[cfg(windows)]` (compiles here), zero callers anywhere; **still present 15 days after 07-05 flagged this exact ~114-LOC cluster.**
- `post_start_drag` (`ui_tasks/drag.rs:315`) — specifically the Windows no-op stub variant; its only potential caller is itself gated to non-Windows, so this stub is unreachable in principle on this platform, not just today.

**Full-function dead:**
- `build_config_files_with_bus` (`agent_config.rs:153`) — see §1.3, zero callers, also a latent regression if ever wired up.

### 2.2 UNCLEAR — abandoned-mid-flight scaffolding, needs a roadmap decision (~20 items, selected highlights)

These are not simple dead code — each is a documented, intentional piece of a feature that was started and never finished wiring:

- **`Bootstrapping` lifecycle variant** (`reducer/mod.rs:52`) — `HostState` always initializes directly to `Running`; the pre-init gate this variant implies was never built.
- **`Spawning` process-state variant** (`state.rs:330`) — same shape: `handle_register` always inserts `Running` directly, `Spawning` is never produced anywhere, even in tests.
- **`container.rs`'s `stop`/`remove`** (`:311`) — container create/start/exec are fully wired; stop/remove aren't hooked into any agent-delete/pane-close flow yet. `remove()` has exactly one caller, a `#[cfg(test)] #[ignore]` Docker-gated integration test.
- **`BestEffort` process-tracker variant** (`process_tracker/mod.rs:96`) — doc earmarks it for a macOS descendant-escape tracker, but no macOS implementation exists anywhere in the module on any platform yet — not a cfg-gate miss, genuinely unimplemented.
- **`MessagingBridge::send`** (`messaging/mod.rs:137`) — trait method intended for polymorphic dispatch by a `handle_status` aggregation loop that itself doesn't exist anywhere in the crate; no `impl MessagingBridge` exists either. Pure scaffolding for an undescribed feature.
- **`LoginFailure` auth-pattern variant** (`identity/auth_patterns.rs:27`) — matched defensively downstream as if reachable, but no per-provider pattern matcher actually produces it; login-failure detection looks unimplemented/descoped relative to what a wired-up version would need.
- **`AccountNotFound` resolver variant** (`identity/resolver.rs:257`) — never constructed anywhere including tests; doc implies an account-lookup path should produce it, worth checking whether that path now returns `Option` instead, orphaning the variant.
- **`conn_id` field** (`reducer.rs:50`) — populated with real values at multiple production call sites for a stated "log correlation" purpose, but no reducer code actually reads it in any tracing call — a dropped wiring step, not incidental cruft.
- **`instance_id` field** (`agents/runner.rs:56`) / **`InvalidRef` variant** (`agents/runner.rs:64`) — both tied to a documented "Phase 1.5 PR 3" drone-inspector feature and ref-validation path that don't exist yet.

**Recommendation for this whole bucket:** don't delete opportunistically — each needs a 1-line check with whoever owns that roadmap item (still planned vs. quietly descoped). Listed here so the decision is at least visible in one place.

---

## 3. Rust-side code duplication (RPC handlers, storage layer, seed mechanisms)

Full per-file findings run to ~35 items across every `agentmux-srv/src/server/app_api/*.rs` and `agentmux-srv/src/server/*_handlers.rs` file; summarized by consolidation value. All are additive/mechanical extractions (helper functions), none require behavior changes beyond fixing the drift already flagged.

### 3.1 Consolidate now — exact or near-exact duplicates, small/cheap, several already show real drift

| Pattern | Sites | Where |
|---|---|---|
| `WaveEvent{event:"X:changed",...}` broadcast boilerplate | 10+ | `skill.rs`, `mcp.rs`, `bundle.rs`, `drone_handlers.rs`, `identity.rs` |
| Bridge-not-initialized 503 guard | 4 | `messaging_handlers.rs` (Discord/WhatsApp/Telegram/Slack) |
| `bridge.send(msg)` → HTTP response mapping | 4 | `messaging_handlers.rs`, byte-for-byte identical across all 4 |
| `install_chunk` WaveEvent construction | 5 | `install_handlers.rs` — **2 of 5 sites hand-inline the struct instead of calling the already-extracted `emit_line` helper designed for exactly this reuse** — a missed-reuse drift, not just parallel invention |
| Block-lookup-by-id-else-`BLOCK_NOT_FOUND` | 4+ | `agent_io.rs` ×2, `native_memory_handlers.rs` ×3, `shell_handlers.rs` ×1 |
| `get_controller`-else-`NOT_RUNNING` | 2 | `agent_io.rs` (agent.send, agent.stop) — identical error string, easy to accidentally diverge |
| `session_id`-from-block-meta extraction | 2 | `agent_io.rs` — hand-rolls what an `obj::meta_get_opt_string` would do (doesn't exist yet; every other optional-meta-read in these files uses the non-optional `meta_get_string`) |
| Filename path-separator validation | 2 | `blockfile.rs` (`read_state`/`write_state`) — security-relevant validation category, worth having one definition |
| MCP config-JSON + reserved-name (`"agentmux"`) validation | 2 | `mcp.rs` — byte-identical including the FORBIDDEN literal |
| `config_dir` resolution for an agent | 3 | `native_memory_handlers.rs` — **a more complete helper (`memory_dir_for_agent`) already exists in the same file and isn't called by any of the 3 sites that reimplement half of it inline** |
| env-var-else-settings.json resolution | 2 of 3 | `voice.rs` — **the generic helper `resolve_path` already exists right below both non-conforming call sites and is simply unused by them**; the strongest "already generalized but never adopted" finding in the audit |
| `AckResp` success/failure envelope | 2 | `identity_handlers.rs` (auth.cancel, auth.submitcallback) |
| Home-dir path-boundary check | 6 | `editor_handlers.rs` — **real drift already present**: 2 of 6 sites additionally reject "path == home itself" and only 1 of 6 handles a not-yet-existing target path; all 6 claim to enforce the same policy |
| Plain-filename validation (no separators/`.`/`..`/empty) | 3 | `editor_handlers.rs` — identical boolean expression, easy typo trap for a 4th caller |

### 3.2 Larger near-duplicates — consolidation valuable but structurally harder (worth doing, not urgent)

- **`compute_and_ensure_bundle_dir` vs `compute_and_ensure_account_dir`** (`identity_handlers.rs:994-1088` / `:1102-1183`) — ~90 lines each, ~80% textually identical, second explicitly documented as "Direct-account sibling of" the first (i.e. knowingly copy-pasted to avoid touching the original). This is exactly the kind of duplication that drifts silently on the next edit to only one side.
- **OAuth pipe-vs-PTY confirm→persist→finish success/failure sequences** (`identity_handlers.rs`, 4 sites, ~40-100 lines each) — comments explicitly acknowledge "mirrors the pipes path" throughout; **the two post-exit failure-message strings have already diverged** ("CLI exited 0 but authentication check failed" vs "CLI exited cleanly but auth-check still failed") — real, live message-text drift between the two OAuth flavors (pipe-based providers vs. PTY-based, currently only OpenClaw). The sync-vs-async I/O model difference (tokio streams vs. `spawn_blocking`) makes a literal shared function awkward without restructuring; recommend extracting just the decision *body* (confirm/persist/finish calls) into a shared async helper both call from their own exit-status match arms.
- **`generate_subagent_name` vs `generate_dispatch_name`** (`session.rs:215-281` / `:308-372`) and **`register_session_activity_summary` vs `register_session_next_prompt_suggestion`** (`session.rs:374-461` / `:471-553`) — ~55-80 lines each, near-full-body duplicates, but both pairs' doc comments explicitly acknowledge the mirroring ("mirrors `generate_subagent_name`'s admission/semaphore/prompt/block-resolve/haiku-call shape exactly"). The authors were aware and chose not to unify — reasonable to leave unless a 3rd/5th call site appears, at which point the ambient-call admission/semaphore-race boilerplate underneath both pairs (5 near-identical sites, `session.rs:166-512`) is the highest-value single extraction in the whole `app_api` audit if it's ever done.

### 3.3 Explicitly leave as-is (checked, not worth abstracting)

The pervasive `serde_json::from_value(data).map_err(|e| format!("<cmd>: {e}"))?` and `Ok(Some(serde_json::to_value(&x).unwrap()))` idioms recur dozens of times across every handler file audited — these are load-bearing per-call error-context tags, not copy-paste rot; abstracting them would hide the command name from log/error triage for a marginal line-count win. Already-extracted shared helpers (`resolve_agent_definition_id`, `check_s1`, `resolve_tab_id`, `find_agent_block`) are correctly reused everywhere they apply — good examples of prior consolidation working as intended, not new findings.

### 3.4 `skill_seed.rs` / `mcp_seed.rs` — confirmed intentional twin duplication, correctly small

These two modules (built in this session's own recent work) mirror each other closely by design — same shape: parse embedded JSON manifest, `any_starter_X_name_exists`, `seed_starter_X`, all-or-nothing insert with compensating rollback. At 2 call sites this is small enough that a generic `seed_starter_catalog<T>(...)` would trade clarity (obscuring the domain-specific insert calls) for a marginal line-count reduction — correctly left as duplication for now; revisit only if a 3rd seed-mechanism module is ever added.

---

## 4. Rust↔TypeScript mirror-pair audit (beyond the bugs already covered in §1)

Every documented "keep in sync with X" pair found across the two codebases was cross-checked value-for-value, not just structurally. Beyond the two drifted-and-buggy pairs already covered in §1.1/§1.3, the remaining pairs:

**Agree exactly (no action needed), confirmed by direct value comparison:**
- `memory_pressure.rs` thresholds (0.15/0.05 free-ratio) vs `SystemStats.tsx`'s `commitColor` thresholds (0.85/0.95 used-ratio) — exact logical complements, locked by a dedicated Rust test.
- `errors.rs`'s 16 `AmxCode` variants vs `catalog.ts`'s `ERROR_CATALOG` — bidirectional 16/16 match, no orphans.
- `layout_types.rs`'s `LayoutNode`/`LayoutNodeData` vs `frontend/layout/lib/types.ts` — field-for-field match; TS's extra UI-only fields round-trip opaquely through Rust's `serde(flatten)` catch-all by design.
- `agents/types.rs` + `failure.rs` vs `frontend/types/gotypes.d.ts` — `FailureClass`'s 11 variants and `AgentEvent`'s 6 tagged variants match exactly. **Caveat worth flagging on its own:** `gotypes.d.ts`'s header states it is hand-maintained (the original Go generator was removed) — a real ongoing drift-risk surface even though nothing has drifted in the sampled types today.
- `drone/types.rs` vs `frontend/app/view/drone/` — `FlowNode`/`FlowEdge`/`DroneDefinition` match exactly.
- Pinned CLI provider versions (Claude `2.1.198`, Codex `0.116.0`, Gemini `0.32.1`) across all 4 locations (`agentmux-srv/providers.rs`, `agentmux-cef/providers.rs`, `frontend/providers/index.ts`, `.github/workflows/container-image.yml`) — genuinely duplicated 4 ways, but **already load-bearing-CI-enforced**: `pin-consistency.test.ts` reads the actual Rust source files as text and regex-asserts equality, not a frontend-only check. Its own header documents the real 2026-07-02 incident that motivated writing it this way (a pin bump missed cef + the workflow file, 13 patch versions stale). Genuine duplication, real mitigation already in place — no action needed.
- `reducer/layout.rs`'s out-of-range insert-index clamp vs `layoutNode.ts`'s `addChildAt` — behavior agrees (both append-on-out-of-range); the Rust test's own comment cites the wrong TS function (`findNextInsertLocation`, a different code path) — cosmetic, not a behavior bug, but worth a one-line comment fix.

**Confirmed drift, not correctness bugs (documentation/comment rot only):**
- `mod.rs:346`'s `enforce_minimized_locks` doc claims to mirror a TS function, `layoutMinimize.ts::enforceMinimizedLocks`, **that no longer exists** — grep-confirmed zero matches in frontend/. The TS size-lock/snap-back model it describes was deliberately deleted in the 2026-07-16/17 "minimize is a display mode" redesign; Rust still implements the abandoned model as backward-compat for unmigrated persisted layout trees, but the doc comment misrepresents it as an active mirror rather than legacy-format compat.

**Confirmed functional gaps (real, but lower severity than §1's bugs since they're additive UX, not silent failures):**
- `insert_node`/`insert_node_at_index`/`move_node` (`mod.rs:599,635,721`) never port `addChildAt`'s direction-flip/branch-flatten normalization from `layoutNode.ts:44-67`. Partially masked for two of the three Rust functions because their reducer arms run `balance_node` afterward (which coincidentally fixes the leaf-flip case, not the branch-flatten case) — but `insert_node`'s own reducer arm (`reducer/layout.rs:172-312`) is the one structural handler that skips `balance_node` entirely, so it has zero mitigation. 7 dedicated TS tests for this exact behavior (`layoutNode.test.ts:82-177`) have no Rust equivalent — the gap is real and untested on the Rust side.
- `sizeFraction`-based proportional split-carving (`layoutTree.ts:518-642`, the documented "ghost-landing" fix, 4 dedicated tests) is **entirely unported** to Rust — `split_horizontal`/`split_vertical`/`split_impl` have no `sizeFraction` parameter at all, and neither do their reducer handlers.

---

## 5. Frontend dead/duplicate code

### 5.1 Delete now

- **Entire `frontend/app/view/agent-def/` directory — 12 files.** Zero importers outside the directory; the block registry (`app/block/block-registry.ts:32`) never registers an `agent-def` view type. The directory's own model file states in a comment: *"the standalone forge widget was removed in v0.33.197. Do not re-register this as a block view."* Superseded by `AgentSkillsModal.tsx`/`AgentMcpModal.tsx`/`AgentIdentityModal.tsx`/`AgentIdentityPanel.tsx`. A stray `"agent-def": "Agent Definition"` entry in `app/block/blockutil.tsx:35`'s `VIEW_LABELS` map should go with it.
- **`app/view/agent/components/FilterControls.tsx`** — zero references anywhere outside its own file.
- **`app/view/agent/components/NewAgentCard.tsx`** — zero references outside its own file; own doc comment says it's a placeholder from a PR-1/PR-2 split where PR-2 shipped the real affordance elsewhere (`AgentPicker.tsx`'s own "+ New") and this was never removed — the exact same "superseded sibling left behind" shape as the `PaneRegions.tsx`/`ForkBar.tsx` pair already deleted earlier this session.
- **`export function IdentityView(...)` inside `identity-view.tsx:46`** — dead export in an otherwise-live file (the rest of the file, `IdentityPanel`/`AccountsTab`/`AccountForm`, is genuinely used). Superseded by `identity-pane-view.tsx`'s `IdentityPaneView`, which is what's actually registered.
- **`.agent-document-node-wrapper` class** (`app/view/agent/styles/_document.scss:126-132`) — own comment: "Legacy class — keep for any non-virtualized callers; remove once Phase 2 is fully wired in everywhere." Zero `.tsx`/`.ts` applies this class; the same file's own other comments confirm Phase 2 is fully wired.
- **`icon: string` field on `BlockKindMeta`** (`app/view/drone/block-registry.ts:25-26`) — own comment: "legacy; superseded by `emoji`." Populated on every registry entry, never read by any renderer.
- **`detectAgentFromPath()`** (`app/block/autotitle.ts:236-245`) — `@deprecated`, only remaining caller is the test that tests the shim itself; no production call site.

### 5.2 Duplicate utilities — consolidate

| Utility | Sites | Note |
|---|---|---|
| Agent env-slug computation (`agent.slug \|\| agent.name.toLowerCase().replace(...)`) | 4, byte-identical | `agent-model.ts` ×3, `buildStartupPayload.ts` ×1 |
| Browser URL normalize-or-search | 2, byte-identical | `browser-model.ts`/`browser-view.tsx` — view pre-normalizes then hands to model which re-normalizes; redundant work, not just duplicated code |
| `formatElapsed` mm:ss duration formatter | 4, 2 byte-identical | one variant (`PersistentShellBlock.tsx`) is missing the `Math.max(0,…)` clamp the others have — a real minor behavior drift |
| "N ago" relative-timestamp formatter | 4, near-identical | `MyAgentsList.tsx`'s version claims in a comment to be "centralized" but isn't actually imported by the other 3 sites — comment doesn't match reality |
| Byte-size formatter | 2, diverging conventions | `ToolOverlayLog.tsx` uses B/KB/MB, `blockstats.tsx` uses K/M/G for the same underlying problem — a UI-consistency issue, not just dedup |

### 5.3 Real bug found alongside the utility audit

Already covered in §1.4 (clipboard bypass) — listed there since it's a correctness issue, not just duplication, even though it surfaced during this pass.

### 5.4 SCSS keyframe duplication

- **360° spin animation**, 2 near-identical definitions: `StatusBar.scss:553` (`status-icon-spin`) and `_control-bar.scss:463` (`agent-spin`). (A third candidate, `_install-modal.scss:55`, was checked and is a genuinely different discrete 4-step flip — correctly not a duplicate.)
- **Opacity pulse animation**, 4 near-identical definitions across `_status-dot.scss`, `_control-bar.scss`, `_maintenance-section.scss`, `swarm-view.scss` — all `0%,100%{opacity:X} 50%{opacity:Y}` with only the two opacity values differing. A single parameterized mixin or CSS-custom-property-driven keyframe would collapse all 4.

### 5.5 Event-subscription boilerplate

No `useWaveEvent*`-style wrapper exists anywhere in the tree. The `onMount(() => { const unsub = waveEventSubscribe({...}); onCleanup(unsub); })` shape is copy-pasted an estimated 20-25 times across 17 files (the `hooks/` directory alone: `useAgentFailure.ts`, `useBlockActivity.ts`, `useControllerStatusEvents.ts`, `useProcessCount.ts`, `usePtyWidth.ts`, `useSubagentEvents.ts`, plus many component-level one-offs). A shared `useWaveEventSubscription(eventType, handler, {scope?})` would collapse the Solid-hook call sites; the ~20 class-based "Model" files (`swarm-model.ts`, `subagent-model.ts`, etc., which use constructor + `unsubs.push()` + `dispose()`) are a structurally different pattern and wouldn't be covered by the same wrapper — lower priority, already fairly uniform as-is.

### 5.6 Catalog/tile-picker pattern — only 2 instances today, not yet worth a shared component

`accounts-catalog.ts`+`AccountsGallery.tsx` and `mcp-preload-catalog.ts`+`McpCatalogPicker.tsx` are structurally near-identical (static array → tile grid → click prefills a form). Everything else checked (OAuth catalog, toolchain/widget catalogs, `AgentPicker`'s template grid) is either a different interaction shape or too diverged (live-DB-backed with real orchestration) to share a component without real design work. **Recommendation: track for a 3rd instance, don't build `<CatalogPicker>` yet.**

---

## 6. Legacy/stale subsystem remnants

### 6.1 Confirmed dead, still present 15 days after the 07-05 audit flagged the general pattern

- **`Taskfile.yml:379-470` — 11 `tsunami:*` tasks** invoking `go run`/`go build` against a `tsunami/` directory that does not exist anywhere in the repo. Directly contradicts CLAUDE.md's own "Tauri, Go, and Electron code has been removed."
- **`scripts/verify-package.sh`** — references `electron-builder`/`make/win-unpacked`/`make/darwin`; not called from `Taskfile.yml`, `package.json`, or CI. Current packaging is CEF/Inno Setup/AppImage.
- **`scripts/benchmarks/measure-performance.{sh,ps1}` + their README** — reference `src-tauri/target/release/...`, the pre-rename product name, and `docs/TAURI_MIGRATION_STATUS.md`, which does not exist.
- **4 `scripts/dev-tools/*.ps1` scripts** (`scroll-console.ps1`, `scroll-console-top.ps1`, `click-console-area.ps1`, `clear-console.ps1`) hardcode `EnumWindows` looking for a window titled `"DevTools - tauri"` — would simply fail to find anything today. Sibling scripts in the same directory (`open-devtools.ps1`, `click-console.ps1`) already use the current `Get-Process -Name 'agentmux'` convention, showing these 4 just weren't updated when the others were.
- **`Taskfile.yml:83-93`'s `storybook`/`storybook:build` tasks** — no Storybook config or dependency exists in `package.json`.
- **`package.json:24`'s `"package:portable"` npm script** — no matching `package:portable` Taskfile target exists; broken today.
- **`package.json:130-137`'s `workspaces` array** lists `"docs"` as a member; no `docs/package.json` exists.
- **`"fromElectron"` argument documented but never sent/read** — `agentmux-srv/src/backend/service.rs:285,353` list it in `arg_names` for `CloseWindow`/`CloseTab`, but neither the real Rust handlers (`window.rs:387-391`, `workspace.rs:401-409`) nor the frontend callers (`services.ts:105-106,139-141`) ever pass it. Harmless (RPC-introspection metadata only) but factually wrong.
- **Stale comments referencing a deleted `initTauriApi` function** (`frontend/util/getenv.ts:24`, `frontend/app/store/backendStatus.ts:74`) — real init entry point is `initCefApi()`.
- **"Tauri" branding leaking into live perf/log-tag string literals** — `frontend/app-init.ts:398,437,449,506` (`tlog("TOTAL initTauriWave", ...)`, `sendLog("[initTauriNewWindow] ...")`, etc.). Since `muxlog` is the primary operator-facing log tool per CLAUDE.md, an engineer troubleshooting startup perf today sees "TOTAL initTauriWave" in the trace and could reasonably wonder if Tauri code is still running. Purely cosmetic (just string labels) but worth renaming to `initCefWave`/`initCefNewWindow`.

### 6.2 The most externally-visible unmigrated rename: `preset.*` survives as the *primary* name on the MCP tool surface

Every other surface in the preset→bundle rename (App API commands, frontend RPC bindings) has `bundle.*` as the primary identifier with `preset.*` demoted to a documented, explicitly-temporary alias (`agentmux-srv/src/backend/rpc_types/commands.rs:298-310`: *"Deprecated 'preset.*' aliases — kept wired for one release (remove in Phase 4)"*). The one place that never got renamed at all:

- `agentmux-mcp/src/main.rs:357-372` — the MCP tools every spawned agent sees in its own tool list are literally named `PresetList`/`PresetGet` (descriptions still say "A preset is a provider-agnostic config bundle...").
- `agentmux-mcp/src/main.rs:1537-1580` calls `GET /api/v1/agent/preset/list`/`/preset/get`.
- `agentmux-srv/src/server/mod.rs:332-333,754,834-861` — the underlying REST routes and handler function names (`handle_agent_preset_list`/`handle_agent_preset_get`) are still `preset`-named, non-deprecated, no alias.

Separately: per `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`'s own Phase 4 sequencing table, sub-items 4a (table rename), 4b (UI removal), 4c (`db_identity_bundles` drop) have all already shipped — confirmed via `agentmux-srv/src/backend/storage/migrations.rs:130-144` (explicit `DROP` of `db_identity_bundles`/`db_identity_bindings`, both fully gone, no live references remain anywhere outside explanatory doc comments on the now-vestigial `identity_id` field). Only 4.4 (removing the `preset.*` App API aliases) has not happened, despite the spec being 18 days old as of this report and its siblings all landed.

**Recommendation:** rename the MCP tool surface (`PresetList`→`BundleList`, etc. — the highest-visibility miss, since it's what every agent's own tool list shows) and check with the refactor's owner whether the "one release" compat window for the `preset.*` App API aliases has now elapsed.

### 6.3 Other rename-hygiene findings

- **`frontend/app/view/memory/memory-model.ts:4-12`'s file header** still says `// User-facing name is "Presets"` — the shipped UI says "Bundles" everywhere (`armory-view.tsx:20`, `memory-model.ts:133,180` itself, `memory-manager.tsx:277`). Stale doc comment, minor but real drift within the same file that contradicts its own runtime behavior.
- **`bundle_memory_*` method names intentionally NOT renamed** — confirmed as a deliberate, documented product decision (CLAUDE.md explicitly overrides the original spec's plan to rename these) rather than an oversight. Not a finding — noted so a future pass doesn't misflag it.
- **`db_identity_bundles`/`db_identity_bindings`** — confirmed fully gone; the PR-A/B/C sequence's specific ask is clean.
- **`agentbus`→`muxbus` naming** — confirmed clean; all `agentbus` string hits are in `specs/` (explicitly grandfathered per CLAUDE.md) or are false-positive substring matches, not live code.

### 6.4 Duplicate constants without an automated guard

- **`startup-splash.ts:17-18`'s `FADE_MS = 200` vs `index.html:38-42`'s `transition: opacity 200ms`** — both sides carry a "keep in sync" comment pointing at each other, but unlike the CLI-pin-version quadruple (§4, which has `pin-consistency.test.ts`), **nothing enforces these stay equal.** Currently consistent, but this is exactly the pairing shape that silently desyncs on a future edit to only one side. Low severity (a genuinely wrong value here just means a slightly-off fade duration, not a crash), but cheap to fix properly — inject `FADE_MS` into a CSS custom property from JS instead of hand-duplicating the number.

### 6.5 Orphaned specs whose own text says they're superseded, not yet archived

- `docs/specs/SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md` — header literally states `**Status:** SUPERSEDED by the display-mode model, implemented 2026-07-16`.
- `docs/specs/SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md` — has its own "Status update... implementation diverges" section marking most of the doc's body historical.

Both are clean candidates for `docs/specs/archive/`, matching how e.g. `docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` was already handled.

---

## 7. Suggested prioritization

1. **Fix the 5 real bugs in §1** — these are correctness issues, not hygiene, and none are test-covered. §1.2 (data loss) and §1.1 (silently-swallowed API failures) are the two worth treating as genuine bugs rather than backlog items.
2. **Delete the confirmed-dead items in §2.1 and §5.1** — mechanical, zero behavior risk, ~30 Rust items + 1 frontend directory + 3 components. Good first PR.
3. **Rename the MCP tool surface off `preset.*`** (§6.2) — the single highest-visibility unmigrated-rename finding; every agent's tool list shows this today.
4. **Small consolidations from §3.1/§5.2/§5.4** — cheap, low-risk, several already show real string/behavior drift between "identical" call sites, which is itself a reason to do them rather than leave them.
5. **Everything in §2.2 (UNCLEAR Rust scaffolding)** — not a cleanup task, needs a roadmap conversation per item; listed for visibility, not immediate action.
6. **Larger consolidations (§3.2), event-subscription hook (§5.5), remaining §6 legacy-script deletions** — real value, lower urgency; good candidates for a dedicated cleanup PR once §1-4 land.

---

## 8. References

- `docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md` — predecessor audit; several findings here (`wndproc.rs` frameless cluster, `resolve_provider_alias`, general Tauri/Go/Electron remnant pattern) were independently re-confirmed present, still unfixed 15 days later.
- `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` — source of the Phase 4 sequencing referenced in §6.2/§6.3.
- `docs/specs/SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md` — prior architecture-health pass, not re-derived here.
- `docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md`, `docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md` — same-session prior reports, unrelated scope, cited only for the `docs/specs/REPORT_*.md` naming convention this report's sibling location (`docs/reports/`) predates and coexists with.
- Internal file:line citations throughout are as captured 2026-07-20 against `main` @ v0.54.2; verify against current source before acting; some volume (§3/§4/§5) came from parallel research passes and was spot-verified rather than independently re-read line-by-line by the report author for every citation.
