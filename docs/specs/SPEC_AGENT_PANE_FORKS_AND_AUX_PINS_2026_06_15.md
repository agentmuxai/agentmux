# SPEC: Agent pane forks + a cohesive auxiliary-pins architecture

**Date:** 2026-06-15
**Status:** Draft — architecture + best-practices (no code landed)
**Scope:** Agent pane (`frontend/app/view/agent/**`), block/layout model, agent runtime
**Related:** `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md` (the top dock that just
shipped), `specs/archive/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md` (AgentInstance + fork),
`SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md`, `SPEC_ASK_USER_QUESTION_2026_06_15.md`
(validation-gate discipline reused here)

> **Naming.** These in-pane conversations are **forks**, not "layers." The word matches the
> codebase's own vocabulary (`ForkAgentDefinitionCommand`, `parent_id`, `branch_label`) and
> Claude Code's `--fork-session`; it names the actual relationship (branched from a parent,
> then diverges) rather than a spatial metaphor; and it avoids colliding with the pane's real
> *layers* — the z-stacked overlays (focused panel, pane-scoped modals, `usePaneOverlay`
> clipping). The bottom strip is the **fork bar**; each row is a **fork**; the very first
> conversation is the **root** (rendered as the base — like git's `main` is a "branch" too).

---

## 1. Intent

Two user-facing asks, plus one architectural ask that ties them together:

1. **Top aux pins (shipped):** long-running processes (shells/crons/subagents) pin to the
   top of the agent pane — the **ActivityDock**. This already exists.
2. **Bottom aux pins (new):** below the text input, a row of pins where **each row is a
   running agent/conversation *within the same pane*.** Pressing **Down** (or clicking)
   **loads that conversation** — the Claude-Code "press ↓ to jump to a conversation"
   gesture, realized as in-pane **forks**. Forking should be one keystroke — e.g. a
   `/btw` that spins a side thread off the current conversation.
3. **The meta-ask:** make **all** auxiliary pins (top processes + bottom forks + the other
   strips/banners) one **cohesive architecture** instead of a stack of bespoke surfaces —
   and decide whether the current approach needs a rethink.

This doc is the best-practices/architecture spec for all three. It deliberately separates
the **unifying accessory model** (§5 — applies to every pin/strip) from the **forks
feature** (§6–8 — the new capability), because the first is reusable infrastructure and the
second is one consumer of it.

---

## 2. Current state (what we actually have)

**Runtime: one pane = one block = one conversation.**
- A pane is a layout node holding exactly one `blockId`; the block owns one controller
  (`SubprocessController` / `PersistentSubprocessController`). There is no
  multiple-conversations-per-block today (`frontend/types/gotypes.d.ts` Block; the
  `subblockids` array is unused for agents).
- A conversation's identity is the CLI **session id**, captured from the init event and
  stored in `block.meta["agent:sessionid"]` (`subprocess.rs` / `persistent.rs`). Transcript
  bytes live in a **global zone keyed by the agent *definition* id**:
  `agent:<defId>:current` (`agent_session.rs`), with `output` (NDJSON) +
  `output.state.json` (snapshot), mirrored to the cross-channel store.
- **Crucially: one `:current` zone per definition.** Two live conversations of the *same*
  definition would collide on that zone. Archived conversations move to
  `agent:<defId>:archive:<ts>`.

**Sessions, instances, forks (already modeled).**
- `AgentInstance` (DB row) is the per-launch session record:
  `{ id, definition_id, parent_instance_id, block_id, session_id, status, identity_id,
  memory_id, instance_name, working_directory, … }` — it already carries **lineage**
  (`parent_instance_id`) and binds a session to a block. This is the natural **fork** entity.
- **Resume / reattach** is solved: the picker's `RecentSessionRow` → `launchAgentDefinition`
  with `continueOfInstanceId` + `continueSessionId` → `block.meta["agent:sessionid"]` →
  `--resume <sid>` on first turn.
- **Forking** is solved at the *definition* level: `ForkAgentDefinitionCommand` makes a new
  `AgentDefinition` with `parent_id` + `branch_label`; the picker shows a fork prompt when a
  definition is already open. Today a fork opens as a **separate block/pane**, not an in-pane
  fork in the same pane.
- **`claude --fork-session`** exists: "when resuming, create a new session ID instead of
  reusing the original" (verified `claude --help`, 2026-06-15). Plus `--session-id <uuid>`,
  `-c/--continue`, `--from-pr`. So branch-a-conversation is a supported CLI primitive — the
  exact mechanism an in-pane fork needs.
- **`/btw` does not exist** as a command today.

**The top dock (the pattern to generalize).**
- `ActivityDock` + `ActivityRow` render pinned long-running activities. The data model is a
  **pure derivation**: `shellActivities(documentNodes)` maps every `ShellNode` in the
  conversation doc → `PinnedActivity { id, kind, title, status, startedAt, endedAt, canStop,
  shell }` (`activity/shell-adapter.ts`, `activity/types.ts`). The dock owns only *view*
  concerns: retention (D4), ordering (D3), overflow (D6), expand/collapse via
  `documentState.pinnedNodes`. No duplicated state. This "derive pins from a source of
  truth" discipline is the model to keep.

**The accessory stack (the part that's ad-hoc).**
The agent pane render tree (`agent-view.tsx` `AgentPresentationView`) hand-stacks ~16 fixed
surfaces in flex order, each bespoke, no shared base:

| region (today, implicit) | surfaces |
|---|---|
| top fixed | progress bar, search bar, session-digest banner |
| top dock | **ActivityDock** (processes) |
| scroll | document view |
| alert zone (above input) | working row, retry bar, **decision panel**, **question panel**, pending-messages, disconnected banner |
| status | composer strip |
| input | details (control bar + activity log), slash help/picker, footer textarea |
| overlay | focused panel (settings), modals (pane-scoped `ModalLayer`) |

Each has its own SCSS partial; they share spacing/color tokens but **no base component, no
declared region map, no shared pin/row chrome**. New surfaces are added by hand-editing the
JSX order and inventing new classes. That's the thing to formalize.

---

## 3. The asks, precisely

- **A. Forks.** A pane can hold a **set of related conversations** (forks). One is active and
  rendered; the rest are dormant but listed. **Down / click** loads a fork.
- **B. Fork action.** From the active conversation, **`/btw`** (and/or a `+` affordance)
  **forks** it into a new sibling that shares history up to now and then diverges.
- **C. Fork bar.** A row **below the input** lists the forks (active highlighted), with status
  color + title + switch + close; this is a *consumer* of the unified pin model.
- **D. Cohesion.** Top dock (processes) and bottom bar (forks) — and ideally the other
  strips — share one accessory architecture.

---

## 4. Does it need a rethink? — verdict: **a targeted one, not a rewrite**

**Keep (it's sound):**
- The **runtime**: one block = one conversation = one controller = one transcript zone. It's
  proven, crash-recoverable, and cross-channel. **Forks must not break it.**
- **AgentInstance** as the session/lineage entity — it already is the "fork".
- The **data-derived pin** discipline from ActivityDock.
- `--resume` / `--fork-session` / `continueOfInstanceId` plumbing — reuse verbatim.

**Rethink (formalize what's ad-hoc):**
1. **Promote the implicit flex stack to a declared *region* model** (§5.1) so surfaces
   register into named slots instead of being hand-ordered in one 300-line JSX block.
2. **Extract one shared pin/row primitive** (§5.2) from `ActivityRow` so the top dock and
   the bottom fork bar (and future strips) share chrome, status colors, retention, overflow.
3. **Introduce *forks* as a block-stack at the pane level** (§6) — the one genuinely new
   structural concept — chosen specifically because it *reuses* the runtime rather than
   multiplexing it.

**Reject (the tempting-but-wrong rethink):** do **not** make one block host N live
conversations/controllers (multi-session block). It would re-key transcript zones, multiplex
controllers, and fork the crash-recovery model — high blast radius into the proven runtime
for no gain over the block-stack approach (§8).

---

## 5. The unifying model — **Pane Accessories**

A small, shared vocabulary every auxiliary surface opts into. Two concepts: **Regions**
(where) and **Pins/Rows** (what).

### 5.1 Regions — a declared slot map per pane

Replace the implicit flex order with a named region map rendered by one container. Top→bottom:

```
┌ region: top-fixed     progress · search · digest                    (transient banners)
├ region: dock          ActivityDock — processes pinned, sticky        (top aux pins) ✓ ships
├ region: stream        the conversation (flex: 1, scrolls)
├ region: alert         working-row · decision · question · disconnected  (one-at-a-time-ish)
├ region: queue         pending-messages
├ region: status        composer strip
├ region: input         details · slash · footer textarea
└ region: forks         Fork bar — conversations in this pane          (bottom aux pins) ★ new
   region: overlay       focused panel · pane-scoped modals (z-stacked, clipped — the *real* layers)
```

- A region is a flex slot with a fixed contract: `flex: 0 0 auto` (or `1` for `stream`),
  `flex-shrink: 0`, an own `max-height`, and a z-order. Regions are **declared once**;
  surfaces *register into* a region rather than being positioned by JSX accident.
- **`forks` is the new bottom-most region**, below `input` — matching "below the text input".
- The `overlay` region keeps the existing `usePaneOverlay()` native-pane clipping +
  `ModalLayer scope="pane"` mechanism unchanged. (This — not the fork bar — is what "layers"
  legitimately means in this pane.)

Implementation note: this can be a thin `<PaneRegions>` component that takes a record of
`region → JSX[]`, so the giant ordered JSX in `agent-view.tsx` becomes a declarative map.
It is a *refactor with no behavior change* (Phase 1) — the safety bar is "pixel-identical".

### 5.2 The Pin/Row primitive — generalize `ActivityRow`

Extract a `<PaneRow>` (rename-agnostic; today's `.agent-activity-row` chrome) consumed by
both the dock and the fork bar:

```ts
interface PaneRow {
  id: string;
  sigil: string;                 // ⟩ shell · ⟳ cron · ◆ subagent · ⑂ fork (active: ▣)
  title: string;
  status: "running" | "active" | "idle" | "done" | "error" | "stopped";
  meta?: string;                 // elapsed, token totals, "3 msgs", branch label…
  tail?: string;                 // latest log line (dock) or last user msg (fork)
  actions?: PaneRowAction[];     // stop ■ · dismiss × · switch ↵ · close ⌫
  expandable?: boolean;          // inline expand (dock: live log; fork: nothing or preview)
  accent: StatusColor;           // drives the 3px left border, per existing _shell-node.scss
}
```

Shared conventions (lift from the dock, make them the house rules for *all* pin rows):
- **Status accent** = 3px left border, color by status (running=green, error=red, …) —
  already in `_shell-node.scss`; promote to a `_pane-row.scss` base + a `@mixin pane-row`.
- **Ordering (D3)**, **retention (D4)**, **overflow (D6 — "▸ N more")**: the dock's rules
  become the documented defaults; a row source declares whether it opts in.
- **Cursor**: rows are interactive → `var(--cursor-interactive)`; non-actionable meta uses
  `var(--cursor-default)` (per the cursor-token work, `ANALYSIS_CURSOR_STYLING_2026_06_15`).
- **Derive, don't duplicate**: a pin source is a *pure function of a source of truth*
  (dock ← document ShellNodes; forks ← the pane's instance set), never a parallel store.

### 5.3 Best-practice rules (the doc part)

1. **One source of truth per pin family.** Pins are derived; never hand-mutated alongside the
   thing they represent.
2. **Surfaces register into a region; they don't choose pixels.** Order / z-index / max-height
   are the region's contract, not the surface's.
3. **Reuse `<PaneRow>` chrome** for anything pin-shaped (dock, forks, future: queued jobs,
   warden alerts). Bespoke chrome only when genuinely not row-shaped (e.g. the composer).
4. **Alert region is scarce.** decision/question/disconnected are mutually-exclusive-ish,
   modal-weight surfaces; cap concurrency and queue, don't stack unboundedly.
5. **Validation-gate provider behavior** before building on it (the AskUserQuestion §10
   discipline) — see §6.4 for the fork smoke test.

---

## 6. Agent pane forks — the feature

### 6.1 Model: a **fork = an existing block** (no runtime change)

> A **fork** is a `blockId` (its own `AgentInstance`, its own controller, its own
> `agent:<defId>:current` transcript). A **pane hosts a *fork set*** — an ordered list of
> blockIds with exactly one **active**; the pane renders the active block's
> `AgentPresentationView`, the others stay mounted-but-dormant (or suspended). The first
> conversation is the **root**; the rest are forks of it (or of each other).

This is the whole trick: **a fork is just a block**, so every hard problem (session capture,
resume, transcript persistence, crash recovery, controller lifecycle) is **already solved**.
The only new structure is "a layout node can hold N blocks, one visible" + the switch UI.

### 6.2 The one structural change: block-stack at the layout node

- Extend the **layout node** (not the block, not the controller) to hold a
  `blockStack: string[]` + `activeBlockId`, defaulting to a single-element stack (100%
  back-compat — every existing pane is a 1-fork stack, i.e. just its root).
- The tile renderer renders `activeBlockId`; dormant forks are kept in the store (and may
  keep their controller alive in the background, or be *suspended* — see §6.6).
- Stack membership + active id persist in layout/tab state (same durability as today's
  panes), so a pane reopens with its forks intact.
- This isolates the change to the **layout store** (`agent-pane-layout-store` + the tile
  layout) and the **fork bar UI**. The agent runtime is untouched.

### 6.3 Forking (`/btw` and the `+` affordance)

- **`/btw <note?>`** — a new slash command (`commands/global/`): "by the way" — fork the
  current conversation into a sibling to explore a tangent without disturbing the main
  thread. Optionally seed the new fork with `<note>` as its first message.
- **Mechanism (reuses everything):**
  1. Fork the definition: `ForkAgentDefinitionCommand(source=activeDefId)` → new def with
     `parent_id` + a `branch_label` (so the fork's transcript zone `agent:<newDefId>:current`
     never collides with the parent's — §2's per-definition-zone constraint is *satisfied by
     construction*).
  2. Create an `AgentInstance` for the new def with `parent_instance_id = activeInstanceId`
     and `continueSessionId = activeSessionId`.
  3. Spawn its block with **`--resume <parentSid> --fork-session`** so it inherits history up
     to the fork point and then diverges with a fresh session id (captured normally on first
     turn).
  4. Push the new blockId onto the pane's `blockStack` and make it active.
- A **`+`** button on the fork bar = "fork current" with an auto `branch_label`.

### 6.4 Validation gate — ✅ PASSED 2026-06-15

Confirmed empirically against the bundled `claude` (one-shot `-p` harness, the AskUserQuestion
§10 pattern). Parent established a fact ("secret word is BANANA"); a fork was spawned with
`claude --resume <parentSid> --fork-session …`:

```
fork inherits parent history     = True      # fork recalled "BANANA"
fork gets a NEW session id        = True      # b5c335de… ≠ parent 22d4aef4…
two forks are independent ids     = True      # two --fork-session children → distinct ids
parent id stable on plain resume  = True      # `--resume` (no fork) keeps the parent id
parent still recalls the fact     = True      # parent untouched by the forks
```

**Conclusion:** `--resume <parentSid> --fork-session` shares history up to the fork point,
diverges with a fresh session id, leaves the parent session intact, and yields independent
children — exactly the §6.3 mechanism. The fork flow is unblocked for Claude.

Non-Claude providers without a fork flag fall back to "fork = new definition, fresh start"
(Phase 2 scope note), surfaced honestly in the UI.

### 6.5 Switching forks (the "press ↓ to load that conversation" gesture)

- The **fork bar** (bottom region) lists forks as `<PaneRow>`s: sigil ⑂ (active ▣), title =
  branch label / instance name, status accent (active / running / idle / error), tail = last
  user message, actions = switch ↵ · close ⌫.
- **Click a row** → set `activeBlockId` → tile renders that block.
- **Down arrow**, when the composer is empty and the caret is at the start (so it doesn't
  fight text editing), cycles to the next fork and loads it; **Up** to the previous —
  mirroring Claude Code. (Keybinding lives in `useAgentKeyboard`; gated on empty composer.)
- Switching is **view-level** (swap the rendered block); it does not stop/respawn controllers.

### 6.6 Dormant-fork lifecycle (the real design decision)

When a fork is not active, its block still exists. Options, pick per resource budget:
- **Keep-alive (simplest):** dormant controllers keep running (a background turn keeps
  streaming into its own transcript; the fork bar shows its `running` status live). Best
  UX, highest resource use.
- **Suspend-on-blur:** dormant forks stop their controller but keep the transcript; on
  switch, resume via `--resume <sid>` (no `--fork-session`). Cheaper; a switch costs one
  respawn. **Recommended default** for ≥N forks; keep-alive under the cap.
- The fork bar's status accent reflects this: `running` (live, even if dormant), `idle`
  (suspended), `error`. This is exactly the dock's status-accent convention reused.

### 6.7 Persistence & cleanup

- Each fork persists as its own block/instance/transcript (no new persistence code).
- Stack membership persists in layout state; reopening the pane restores the forks and the
  active one.
- Closing a fork = archive its session (`archive_session`, existing) + remove from stack;
  closing the **root / last** fork = closing the pane.
- Forked **definitions** accumulate (like today's forks); reuse whatever pruning the
  definition/instance lifecycle already provides (`display_hidden`, archive).

---

## 7. The fork bar (UI)

- **Region:** `forks`, below `input` (bottom-most non-overlay).
- **Rows:** one `<PaneRow>` per fork in the pane's stack; active row visually promoted
  (filled sigil ▣ + accent background), others muted; the root reads as the base entry.
- **Affordances:** click/↵ switch · `+` fork-current · `⌫` close fork · status accent.
- **Empty state:** a single-conversation pane (root only) shows **no bar** (or a thin `+`
  only) — zero cost for the common case; the bar appears once a second fork exists. (Mirrors
  the dock, which is absent with no processes.)
- **Overflow:** the dock's D6 "▸ N more" rule, reused via `<PaneRow>`.
- **Theming/cursor:** `<PaneRow>` base; interactive cursor; status colors from the existing
  `_shell-node.scss` palette promoted to `_pane-row.scss`.

---

## 8. Architectural decision: block-stack vs multi-session block

| | **Block-stack (recommended)** | Multi-session block (rejected) |
|---|---|---|
| Fork = | a block (existing) | a session inside one block |
| Runtime change | none — reuse controllers/zones/recovery | multiplex controllers in one block |
| Transcript zones | one per fork (per-def `:current`), no collision | must re-key per-session (breaks `agent:<defId>:current`) |
| Crash recovery | unchanged (per-block) | must re-derive per-session |
| Blast radius | layout store + UI only | deep into the proven agent runtime |
| Fork action | reuse `ForkAgentDefinition` + `--fork-session` | bespoke session spawn inside block |
| Cost | one new layout concept (block-stack) | many |

**Decision: block-stack.** It buys the entire feature by adding a single layout-level concept
("a node can hold N blocks, show one") and a bottom bar, while the high-risk runtime stays
exactly as shipped. The per-definition transcript-zone constraint — which would *force* ugly
re-keying under the multi-session model — is instead **satisfied for free**, because a fork is
already a new definition with its own zone.

---

## 9. Phasing

| Phase | Deliverable | Risk | Notes |
|---|---|---|---|
| **1** | **Accessory regions + `<PaneRow>` extraction.** Declare the region map; refactor `ActivityDock`/`ActivityRow` onto `<PaneRow>` + `_pane-row.scss`. **No behavior change** (pixel-identical). | low | Pure refactor; unblocks everything; reviewable on its own. |
| **2** ✅ | **Fork model + fork bar (read-only switch).** Layout node `blockStack`/`activeBlockId`; render active; bar lists *already-related* instances (a definition + its open forks) and switches between them. Shipped as `PaneTabStrip` + `pushBlockOntoStack`/`setActiveBlockInStack`/`closeBlockInStack` (`SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md`). | med | The one structural change (layout store + tile). Gated by a design review of dormant-fork lifecycle (§6.6). |
| **3** ✅ (partial) | **Fork action (`/btw` + `+`).** Validation gate (§6.4) → fork-session spawn → push fork. The `+`-affordance half shipped 2026-08-22 as **quick-fork** (`SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md`, see that spec's 2026-08-22 correction notice) — triggered from the pane's body context menu rather than a dedicated fork-bar "+" button. **The `/btw` slash-command half is still unimplemented** — a real, open follow-up, not covered by quick-fork. | med | Depends on the §6.4 smoke test passing for Claude; graceful fallback for other providers. |
| **4** | **Keyboard nav + polish.** ↓/↑ fork cycling (empty-composer gated), suspend-on-blur, persistence of stack, close/archive, overflow. | low | UX polish on a working base. Still open. |

Each phase is independently shippable; Phase 1 is a clean refactor that stands alone even if
the forks feature pauses.

## 10. Open questions / risks

- **Dormant lifecycle (§6.6):** keep-alive vs suspend-on-blur — pick a default + a cap.
  Affects resource use and whether a background fork can finish a turn unattended.
- **Fork session semantics (§6.4):** must validate `--fork-session` independence before
  Phase 3; define the non-Claude fallback.
- **Layout store reach:** `blockStack` touches the tile layout + saga/tear-off paths
  (tear-off a *fork* to its own tab? out of scope for v1 — forking stays in-pane).
- **Definition sprawl:** every `/btw` mints a definition; confirm the instance/definition
  lifecycle prunes hidden/archived forks so the picker doesn't bloat.
- **Down-arrow ergonomics:** must not fight multiline editing; gate strictly on
  empty-composer + caret-at-start, and make it configurable.

## 11. Non-goals (v1)

- Multi-session-per-block runtime (rejected, §8).
- Tearing a fork off into its own tab/window (later; reuse the tear-off saga).
- Merging/diffing forks, or cross-fork message routing (that's the swarm/muxbus surface).
- Re-skinning the non-row surfaces (composer, modals) onto `<PaneRow>` — they aren't
  row-shaped; only the *region* model applies to them.

---

## 12. Files this would touch (orientation, not a change-list)

- **Regions/PaneRow:** new `components/PaneRegions.tsx`, `components/PaneRow.tsx`,
  `styles/_pane-row.scss`; refactor `ActivityDock.tsx`/`ActivityRow.tsx`; `agent-view.tsx`
  render tree → region map.
- **Forks:** `agent-pane-layout-store.ts` (+ tile layout) for `blockStack`/`activeBlockId`;
  new `components/ForkBar.tsx`; `useAgentKeyboard.ts` for ↓/↑.
- **Fork action:** new `commands/global/btw.ts`; reuse `ForkAgentDefinitionCommand`,
  `CreateAgentInstanceCommand`, the `--resume/--fork-session` spawn path
  (`subprocess.rs`/`persistent.rs` already accept the args via `cli_args`).
- **Lifecycle:** reuse `archive_session`, `AgentInstance.status`/`display_hidden`.
