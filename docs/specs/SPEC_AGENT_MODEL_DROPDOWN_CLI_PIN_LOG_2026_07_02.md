# SPEC — Versioned model dropdowns (CLI-aware), Claude CLI pin-to-latest, single-toggle Log

**Date:** 2026-07-02
**Type:** Implementation spec
**Status:** Ready to schedule
**Owner:** asaf
**Scope:** agent-pane composer strip + provider/CLI registry + activity log UI (frontend + srv).

> Three related asks: **(A)** pin AgentMux's Claude CLI to latest; **(B)** the model dropdown above the
> agent input should list **versioned** models (e.g. "Opus 4.8", "Sonnet 5", "Sonnet 4.6") reflecting
> **what the pinned CLI version actually offers** — as the practice for **every** provider; **(C)** the
> **Log** button should directly expand/collapse the full entries list — remove the redundant middle
> collapse layer.

---

## A. Pin the Claude CLI to latest

### Current state
`pinned_version` is **install-only** and duplicated by hand in two registries that must stay in sync:
- `frontend/app/view/agent/providers/index.ts:156` → `pinnedVersion: "2.1.185"`
- `agentmux-srv/src/backend/providers.rs:146` → `pinned_version: "2.1.185"`

Consumed only at install (`install_handlers.rs:416-419`, `cli_handlers.rs:153,240` → `npm install <pkg>@<pinned>`; empty ⇒ `@latest`). Latest on npm today is **2.1.198**. Nothing compares installed-vs-pinned (the toolchain view compares installed vs npm-`latest`, not vs the pin).

### Change
1. Bump both `pinnedVersion` / `pinned_version` for Claude from `2.1.185` → **`2.1.198`** (current latest; verify at implementation time with `npm view @anthropic-ai/claude-code version`).
2. **Single-source-of-truth follow-up (recommended, small):** the TS and Rust registries duplicating `pinned_version` is a drift trap (it's how stale pins linger). Options: (a) generate one from the other, or (b) add a test asserting they match. At minimum, add a code comment on each pointing at the other so a bump updates both.
3. **Drift visibility (optional):** the toolchain view already fetches installed (`get_cli_version`) and npm-`latest`. Add an "installed ≠ pinned" indicator so a stale pin is visible, not silent.

> Note: keep an explicit pin (not literal `"latest"`) — a floating `@latest` makes builds non-reproducible and defeats the version→models mapping in Part B. "Pin to latest" here means *bump the pin to the current latest*, with a lightweight process to re-bump.

---

## B. Versioned, CLI-aware model dropdowns (every provider)

### Current state
- The registry `providers/index.ts` has `ProviderModel { value, label, default?, description?, aliases? }` and per-provider `models?: ProviderModel[]`. Claude's list is **generic** — `opus`/`sonnet`/`haiku`, labels "Opus"/"Sonnet"/"Haiku", **no version numbers** (`index.ts:185-189`). The `value` is an **alias** the CLI resolves to *its current default* for that family.
- **Two GUI dropdowns hardcode their own lists and do NOT read the registry:**
  - `AgentComposerStrip.tsx:43-47,222-224` — `MODEL_OPTIONS` const (the strip directly above the textarea; this is "the panel above the agent input" the ask names).
  - `AgentControlBar.tsx:44-49,294-297` — `MODEL_LABELS` + hardcoded `<option>`s (the details-region control panel).
  - Only the `/model` **slash command** (`commands/global/runtime.ts:58-72`) reads the registry `models`. So the GUI and the registry already drift.
- Selection flow: dropdown → `updateRuntime({ model })` → `applyRuntimeChange` (`runtime-apply.ts:36-60`, persists `agent:runtime` meta; for Claude force-restarts the idle process to re-apply flags) → `buildRuntimeArgs` (`buildRuntimeArgs.ts:103-112`) emits `--model <value>`.

### The core reality — there is no CLI "list models" surface
No code invokes a CLI to enumerate models, and Claude Code exposes no stable machine-readable model list (`--model` just accepts aliases + concrete IDs; the only introspection that exists is `--version` via `get_cli_version` at `cli_handlers.rs:734-750`, and npm-`latest` via `toolchain.versions`). **So "know what the CLI version actually has available" cannot be a live enumeration call.** The honest, robust design:

> **Curated, version-keyed model catalog** — a per-provider list of the models a given pinned CLI version
> supports, with concrete `--model` IDs and versioned labels — kept in sync with the pin, and (best-effort)
> **validated** against the installed CLI rather than enumerated from it.

### B.1 — Concrete model IDs (required to offer specific versions)
To let the user pick **"Sonnet 5" vs "Sonnet 4.6"** as distinct options, the `--model` value must be the **concrete model ID** the CLI accepts (e.g. `claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5-20251001`), NOT the `sonnet` alias (which collapses to one CLI-chosen version). So each versioned entry carries a concrete `value`.

Design for the registry entry (extend `ProviderModel`):
```ts
interface ProviderModel {
    value: string;        // concrete CLI --model ID, e.g. "claude-sonnet-4-6"
    label: string;        // versioned display, e.g. "Sonnet 4.6"
    family?: string;      // "opus" | "sonnet" | "haiku" (for grouping / the alias fallback)
    default?: boolean;
    description?: string;
    aliases?: string[];   // e.g. ["sonnet-4.6"]
    minCliVersion?: string; // gate: only offer when installed CLI >= this (see B.3)
}
```
Claude's curated list becomes versioned, e.g. (verify exact IDs against the pinned CLI at implementation):
`Opus 4.8` (`claude-opus-4-8`, default), `Sonnet 5` (`claude-sonnet-5…`), `Sonnet 4.6` (`claude-sonnet-4-6`), `Haiku 4.5` (`claude-haiku-4-5-20251001`). Optionally keep one "latest per family" alias entry (`opus`/`sonnet`) for users who want auto-tracking — but the ask is versioned options, so lead with concrete versions.

### B.2 — Dropdowns read the registry (both GUI selectors)
Replace the hardcoded lists in `AgentComposerStrip.tsx` and `AgentControlBar.tsx` with `getProvider(providerId)?.models` (the same read `modelChoices` already uses) so all three surfaces (strip, control bar, `/model`) share one source. This also **generalizes to every provider** — drop the `providerId === "claude"` gate (`AgentComposerStrip.tsx:189`, `AgentControlBar.tsx:165`) and show the picker for any provider that defines `models`. Providers whose model is chosen in their own config (muxcode/openclaw/pi/copilot) keep `models` omitted ⇒ no picker (existing contract, `index.ts:98-104`).

Add `models` to the **Rust** `ProviderConfig` (`providers.rs` — currently absent) only if the model list is needed server-side; otherwise the TS registry remains the single catalog and Rust stays lean. (Decide in Open Questions.)

### B.3 — "What the CLI actually has" — the per-provider practice
Since enumeration isn't available, make the catalog **version-aware and best-effort-validated**:
1. **Version-key the catalog.** Each model entry may carry `minCliVersion`. The dropdown filters to models the **installed** CLI supports, using the already-present `get_cli_version` detection. So on an older CLI, newer models don't show; after a pin bump, they do.
2. **Best-effort validation (optional, per provider).** Where a CLI *does* surface hints (e.g. `claude --help` text, a config file, or a cheap `--model <id> --print` dry-probe that errors on unknown models), add a per-provider `listModels()` adapter that **validates/annotates** the curated list against the installed CLI and logs drift ("catalog lists X but the installed CLI rejects it"). This is validation, not enumeration — the curated list stays the source; the probe catches staleness. Structure it as a per-provider hook so each provider implements what its CLI allows (the "practice for every provider").
3. **Documented sync process.** When `pinned_version` bumps (Part A), the catalog is reviewed against the new CLI's models — reference `docs/providers/PROVIDER_MODELS_EFFORT_SETTINGS_2026-06.md` and update it.

### B.4 — Effort dropdown (leave as-is, note the coupling)
The Effort `<select>` sits alongside Model (`AgentComposerStrip.tsx:226-235`) with `EffortLevel = low|medium|high|xhigh|max`. `--effort` is skipped on Haiku (`buildRuntimeArgs.ts:110-112`; Haiku 400s on `--effort`). If model values become concrete IDs, re-verify the Haiku skip keys off the right thing (family, not the old `haiku` alias). No other effort change in scope.

---

## C. Log button — one toggle, no middle level

### Current state — three levels (confirmed)
1. **L1 — the "Log" button** (`AgentComposerStrip.tsx:237-245`) → `DetailsToggle` → `detailsOpenAtom` → renders `.agent-composer-details` (`agent-view.tsx:1279-1283`) containing **both** `<ActivityLogPanel>` *and* `<AgentControlBar>`.
2. **L2 — `ActivityLogPanel`'s OWN header toggle (THE MIDDLE LEVEL)** — its own `isOpen` signal (`ActivityLogPanel.tsx:25`), a header button + **one-line summary** (chevron + `[tag]` + most-recent entry + count, lines 57-91), auto-expand-on-new-entry (lines 32-41). The full `<For>` entries list only renders when this *second* toggle is open (lines 92-117). So the user clicks "Log", then must click the panel header *again* to see entries.
3. **L3 — per-entry row expand** (`expandedIds`, `toggleExpanded`, lines 26,48-55,103-108) — truncated ↔ full text per row. **Keep this** (the user explicitly wants full entries expandable).

### Change
**Collapse L2 into L1** — a single-file change in `ActivityLogPanel.tsx`:
- Remove the internal `isOpen`/`setIsOpen`, the auto-expand `createEffect` (lines 32-41), `userCollapsed`, `mostRecent`, and the header button + one-line preview (lines 57-91).
- Render the entries `<For>` list **directly and unconditionally** (the panel is already mounted only when the details region is open, `agent-view.tsx:1279`).
- Keep the Log button (L1) as the sole toggle and per-row expand (L3) untouched.

### Decision to resolve: the control bar shares the details region
`AgentControlBar` also lives in `.agent-composer-details` (`agent-view.tsx:1282`), so "Log" currently reveals **both** the log and a control panel. Options:
- **(a, recommended)** Log toggles **only the log entries**; leave `AgentControlBar` where it is (still revealed alongside) — minimal, and the control bar is redundant-ish with the always-visible strip selectors anyway.
- **(b)** Split them: Log reveals only entries; move/retire `AgentControlBar` (its Model/Effort duplicate the strip). Larger; out of scope unless wanted.

### Do NOT touch
`ActivityRow.tsx` / `ActivityDock.tsx` are a **separate** feature (pinned long-running-shell dock, `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK`) with their own expand — unrelated to the Log-button chain despite the coincidentally-shared `agent-activity-log` CSS class.

---

## Open questions
1. **Rust `models`?** Keep the model catalog TS-only (single source; Rust stays lean), or mirror into `providers.rs`? Recommend TS-only unless srv needs to validate `--model` server-side.
2. **Aliases vs concrete-only.** Offer only concrete versions (`Opus 4.8`, `Sonnet 4.6`, …), or also a "latest per family" alias entry? Recommend concrete-led + optionally one "Auto (latest Sonnet)" alias.
3. **Validation depth (B.3.2).** Ship curated+version-gated now, add the per-provider CLI-probe validator as a follow-up, or build the probe now? Recommend curated+gated first; probe as follow-up.
4. **Control-bar coupling (C).** Option (a) or (b)?

## Test plan
- **A:** unit/assert that TS `pinnedVersion` == Rust `pinned_version` for Claude; install path builds `@2.1.198`.
- **B:** dropdown renders the registry's versioned labels; selecting "Sonnet 4.6" sends `--model claude-sonnet-4-6` (assert via `buildRuntimeArgs`); `minCliVersion` filters correctly against a stubbed `get_cli_version`; picker shows for a non-Claude provider that defines `models`.
- **C:** with the details region open, entries render immediately (no second click); per-row expand still works; new entries appear without a header toggle. App-run check on Windows (the composer strip is the live surface).

## Risks / notes
- **Concrete model IDs are provider-version-coupled** — a wrong/stale ID means `--model` fails at launch. `minCliVersion` gating + the best-effort probe (B.3) mitigate; the curated list must be verified against the pinned CLI at implementation.
- **Two dropdowns → one registry** removes an existing drift source (net simplification), but re-verify the Haiku `--effort` skip after values change to concrete IDs.
- **CI:** these are frontend + srv changes — the new `ci-pr.yml` will exercise them; add vitest coverage for the dropdown/registry read.

## Sources
- Registry: `frontend/app/view/agent/providers/index.ts:36-42,96-105,156,185-189`; Rust `agentmux-srv/src/backend/providers.rs:71-83,146`.
- Dropdowns: `components/AgentComposerStrip.tsx:43-55,189,214-246`; `components/AgentControlBar.tsx:44-49,165,294-312`; `commands/global/runtime.ts:58-72`; `runtime-apply.ts:36-60`; `buildRuntimeArgs.ts:50-142`; `types.ts:626,636`.
- CLI introspection: `agentmux-srv/src/server/cli_handlers.rs:635-675,734-750`; `install_handlers.rs:416-419`; `frontend/app/view/toolchain/toolchain-view.tsx:187-234` (#1873).
- Log UI: `components/ActivityLogPanel.tsx:25-117`; `components/AgentComposerStrip.tsx:237-245`; `agent-view.tsx:1267-1283`. Separate (do not touch): `ActivityRow.tsx`, `ActivityDock.tsx`.
- Model IDs (verify against pinned CLI): Opus 4.8 `claude-opus-4-8`, Sonnet 4.6 `claude-sonnet-4-6`, Haiku 4.5 `claude-haiku-4-5-20251001`, Fable 5 `claude-fable-5`.
