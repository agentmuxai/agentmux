# SPEC — Settings pane audit: ranked candidates for new sections/controls

**Date:** 2026-08-19
**Type:** Analysis (audit + proposal — no code shipped yet, except where noted)
**Status:** Draft
**Scope:** `frontend/app/view/settings/` (5 current sections: Appearance, Window & Panes, Terminal, Sounds, Advanced — confirmed live via `settings-view.tsx`'s `RAIL` array; CLAUDE.md's own docs table listing a "Network" section is stale, already corrected once in `SPEC_SETTINGS_PANE_COMPLETION_2026_07_14.md`).

## Purpose

A direct audit of real, already-implemented backend/frontend capability that currently has **no Settings UI at all** — either because it's `settings.json`-only (a user has to know the raw key exists and hand-edit JSON) or because it's a hardcoded constant with a real user-facing motivation for being configurable. Cross-referenced against this repo's own prior audits (`specs/settings-cleanup.md`, `SPEC_SETTINGS_PANE_COMPLETION_2026_07_14.md`) so this doesn't re-propose things already deliberately rejected, and doesn't re-litigate things already fixed since.

## Ranked candidates

### 1. Recording / Input section (voice/mic settings) — fully spec'd separately

See `docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md` for the complete design (engine picker, masked API-key field, whisper-local path config, a "test your mic" flow with a live level meter, and a device picker — the first "test X" interaction pattern in the app). Included here only for ranking context: this is the single highest-value gap found — a fully-shipped feature (3 STT engines, per-pane mic button, hotkey) with literally zero discoverability, including a plaintext API key a user currently has to hand-edit into `settings.json`.

### 2. Agent watchdog thresholds (`term:agentmaxruntimehours`, `term:agentidletimeoutmins`)

- Real, live, currently-shipped safety feature: `agentmux-srv/src/backend/blockcontroller/watchdog.rs` (60s poll loop) kills any agent pane exceeding a configured max wall-clock runtime or PTY-idle duration. Both fields exist with doc comments in `agentmux-srv/src/backend/wconfig/types.rs:91-99` ("0 disables the limit").
- **Zero discoverability**: absent from `schema/settings.json` and `frontend/types/gotypes.d.ts`, and there's no Settings row for either — the only way to set them is hand-editing `settings.json` with keys a user would have to already know exist.
- The watchdog's *effect* is partially visible (an `AgentRuntimeBadge` shows elapsed time on long-running panes, `docs/retro/retro-agent-watchdog-badge.md`), but the *knob controlling whether/when it fires* isn't — a user can watch the badge climb with no in-app way to arm a limit.
- **Note:** `SPEC_SETTINGS_PANE_COMPLETION_2026_07_14.md` (lines 39, 110) says these two keys were "GONE entirely" as of July 14 — they're back live in `types.rs` today, so this is a fresh gap, not a previously-triaged-and-rejected one. Confirm current state before implementing (another agent may be actively touching this area, per this session's own experience with parallel-agent races).
- Why a user wants it: prevents a forgotten long-running agent from burning CPU/tokens for days (the motivating incident in the retro: a PID observed running 10 days).
- **Proposed UI**: two `SettingRow`s in the Terminal section (agent panes are terminal-adjacent conceptually, and it's the natural existing home) — numeric inputs with "0 = no limit" placeholder text, following whatever numeric-input pattern (not slider) the Advanced section already uses for integer settings.

### 3. Messaging bridges (`messaging:discord:*`, `messaging:telegram:*`, `messaging:slack:*`, `messaging:whatsapp:*`)

- Real, fully wired server-side: `agentmux-srv/src/bootstrap.rs`, `agentmux-srv/src/server/messaging_handlers.rs`, ~20 dedicated fields in `types.rs:300-437` (enable toggles, bot tokens, channel/chat IDs, per-bridge target-agent routing).
- **Not in `schema/settings.json` at all** (confirmed by reading the current file in full — zero `messaging:*` entries) and no Settings UI. Four complete external integrations (Slack, Discord, Telegram, WhatsApp) configurable only by hand-writing raw JSON containing bot tokens/secrets.
- This is the most "product-shaped" gap in the whole audit — arguably bigger in scope than the Recording section (4 separate integrations vs. 1), but also bigger in implementation cost (4 distinct per-bridge sub-forms, plus the masked-secret-field infrastructure this spec's companion Recording spec already designs — see its Open Question 1 recommending `MaskedKeyField` be built as a shared `settings-controls.tsx` primitive precisely so this candidate can reuse it).
- **Proposed shape**: a new "Integrations" or "Messaging" rail section, one collapsible sub-block per bridge (Enable toggle + bot token via `MaskedKeyField` + the bridge-specific routing fields), gated behind whichever bridge's `enabled` flag like the Sounds section already gates its own sub-rows. Left as a follow-up spec of its own given the scope — flagged here as the #1 candidate for that next spec, not designed in full in this pass.

### 4. Drag-and-drop file-attach settings (`dnd:enabled`, `dnd:concurrency`, `dnd:agentinserttoken`)

- Live in `schema/settings.json:281-295`: `dnd:enabled` (bool, default true), `dnd:concurrency` (integer ≥1, "max files uploaded concurrently on a multi-file drop, absent = unlimited"), `dnd:agentinserttoken` (bool, default true). Consumed by `frontend/app/view/agent/hooks/useAgentDropAttach.ts`, per `docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md`.
- Zero Settings UI. Lower priority than #1-#3 — narrower audience (only matters to users who drag-drop files into agent panes), sane defaults, but cheap: same `ToggleControl`/numeric-input pattern as everything else, no new primitives needed. Good "small win" to bundle into whichever PR touches the Advanced section next.

### 5. `AskUserQuestion` auto-timeout (`AUTO_TIMEOUT_MS`, `HOVER_HIDE_GRACE_MS`)

- `frontend/app/view/agent/components/AgentQuestionPanel.tsx:36` — `AUTO_TIMEOUT_MS = 30_000`, doc comment: *"Hardcoded per SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.2 — not user-configurable in v1."* Companion `HOVER_HIDE_GRACE_MS = 15_000` (line 44) from the follow-up hover-pause spec.
- **This is a spec-blessed, anticipated follow-up, not an oversight** — `SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md` §5 resolved-questions item 2 explicitly says a configurable override "is a reasonable follow-up if requested later, but nothing here blocks adding one afterward," and names the exact mechanism (a settings-store read replacing the current literal).
- Why a user wants it: unattended/overnight agent runs (the spec's own motivating case) may want a longer timeout so the agent doesn't auto-pick the "recommended" option before a human gets a chance to weigh in on something consequential; a low-stakes-question user might want a shorter one for faster fallback.
- **Proposed UI**: one numeric `SettingRow` in Advanced ("Auto-answer timeout for agent questions") — smallest, cheapest candidate in this whole list; the injection point is already anticipated in the existing code, so this is close to a pure UI-only PR.

### 6. Waiting-tone volume has no UI slider

- `notify:sounds:waiting:volume` is fully wired end-to-end (now centralized via `frontend/app/notification/sound/sound-defaults.ts`'s `DEFAULT_WAITING_VOLUME`, this session's own sound-system cohesiveness fix) but `sounds-section.tsx` never got a `SliderControl` for it — only the master and tool-tones volumes did.
- Narrowest candidate here, but nearly free: same file, same `SliderControl` pattern already used twice in that exact section, no new settings/schema/backend work at all (the setting already exists and is already consumed).

## Explicitly NOT good pickings (already decided, don't re-propose)

- **`network:lan_discovery`** — deliberately kept at `HostPopover.tsx` only, not duplicated into Settings (`SPEC_SETTINGS_PANE_COMPLETION_2026_07_14.md` correction note). Re-proposing needs a strong new reason to overturn that explicit call.
- **`tab:preset`, `window:dimensions`, `app:dismissarchitecturewarning`** — deliberately excluded with stated rationale (zero consumers / write-only internal state / one-shot dismiss flag), same source.
- **A long tail of confirmed-dead `SettingsType` fields** (`window:showmenubar`, `window:nativetitlebar`, `window:disablehardwareacceleration`, `window:maxtabcachesize`, `window:confirmclose`, `window:savelastwindow`, `window:zoom`, `app:globalhotkey`, `widget:showhelp`, `telemetry:enabled`, `preview:showhiddenfiles`, `term:localshellpath`, `term:localshellopts`, `conn:wshenabled`, `conn:askbeforewshinstall`) — zero read sites, per the same completion spec's 2026-07-15 correction. Not settings candidates; if anything, candidates for a future *removal* PR, not a UI-addition one.
- **Generic internal-tuning `_MS` constants** (hover-peek delays, animation durations, debounce windows scattered across `ToolBlock.tsx`/`MarkdownBlock.tsx`/etc.) — none have a documented user-facing motivation or a spec flagging them as a deferred setting, unlike #5 above. `SPEC_UNIFIED_TOOL_HOVER_OVERLAY_2026_05_13.md:157` explicitly considered and declined making its own `150ms` hover delay configurable. These read as pure internal feel-tuning, not settings gaps — excluded from this ranking on purpose.

## Suggested sequencing

1. **#1 (Recording/Input)** — already fully spec'd, highest value, ready to implement independently.
2. **#6 (waiting-tone slider)** — trivial, bundle into the same PR as #1 if convenient (same file), otherwise its own tiny PR.
3. **#5 (AskUserQuestion timeout)** — smallest standalone win, spec-anticipated injection point.
4. **#2 (agent watchdog thresholds)** — moderate size, real safety value, needs a "is this still live" re-check first (flagged above).
5. **#4 (drag-and-drop settings)** — small, low-audience, easy to bundle whenever Advanced gets touched next.
6. **#3 (messaging bridges)** — largest in scope, deserves its own dedicated spec before implementation; depends on the shared masked-key-field primitive #1 introduces.

## References

- `docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md` — full design for candidate #1.
- `specs/settings-cleanup.md` (2026-05-11) — original dead-key audit.
- `docs/specs/SPEC_SETTINGS_PANE_COMPLETION_2026_07_14.md` — corrected catalog; source for the "explicitly not good pickings" section above.
- `agentmux-srv/src/backend/blockcontroller/watchdog.rs`, `docs/retro/retro-agent-watchdog-badge.md` — candidate #2.
- `agentmux-srv/src/bootstrap.rs`, `agentmux-srv/src/server/messaging_handlers.rs` — candidate #3.
- `docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md` — candidate #4.
- `docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md` — candidate #5.
- `frontend/app/view/settings/settings-view.tsx`, `settings-model.ts`, `settings-controls.tsx`, `sections/sounds-section.tsx` — current Settings-pane structure and conventions every candidate above follows.
