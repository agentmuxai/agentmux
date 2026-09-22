# SPEC: Permanently blank agent panes after recovery — the hoist deadlock, and the wrong-tab registration that triggers it

**Date:** 2026-09-20
**Status:** proposed — §3.1's non-keep-alive claim CORRECTED 2026-09-20 (see
erratum immediately below); §2, §3.2, §5, §6, §7, and §10 are independently
evidenced (logs/code) and unaffected. §3.1 (keep-alive variant), §3.3, and
§8.1 need rework before implementation — do not implement Layer A against
the original §3.1 text.
**Author:** Lark
**Repo state:** `main` @ `1e976aac8` (v0.56.9); incident observed live on a running v0.56.7 instance (srv pid 27512, host `narko`)

> **Erratum (2026-09-20, same day, before merge):** §3.1 originally claimed
> `hoisted()`/`chromeVm()` deadlock in the **non-keep-alive** path too — i.e.
> for an ordinary, non-stacked single-block pane, which is what `Naki #2`,
> `Loap #2`, and the five Tab-2 agents in this incident all are. **That
> specific claim is wrong, confirmed empirically, not just reread:** a
> targeted test (`pane-leaf-chrome.test.tsx`, "RCA verification (non-keep-alive
> hoist path)") mounts a pane whose view type is already `"agent"` on the very
> first synchronous render — exactly the claimed trigger condition — using the
> file's own realistic signal-backed `NodeModel` double, and **it passes**:
> chrome resolves in 2ms (CI run
> `35539258553`, job `106153809703`, PR #3459). The reason, on rereading:
> `content` (`:327`) is a plain, eagerly-constructed `const` containing
> `<Block>` — SolidJS runs a component's effects at construction time
> regardless of which `<Show>` branch later consumes the resulting value, so
> `<Block>`'s `createEffect` (which publishes the ViewModel) fires whether or
> not `hoisted()` is already true. There is no deadlock here. §3.1's text
> below is kept as originally written, for the investigative trail, but is
> superseded by this note — do not treat it as current. The keep-alive
> variant (`viewModelSlots`) is a distinct code path this test does not
> exercise; whether a live variant of that one still applies is open, tracked
> in §3.3-erratum below. The live symptom itself (5+ panes permanently blank,
> backend data intact) is not in question — only this proposed mechanism for
> it. See `docs/specs/SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md` §8 for
> the last time this codebase prematurely closed a theory about this exact
> symptom class without a disproving test — this erratum exists so this spec
> doesn't repeat that in the other direction (closing on an *unproven*
> theory instead of a disproven one).
**Related:**
`docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`
(same symptom class — dead layout space — different mechanism; its systemic write-path
findings are re-verified as STILL OPEN in §7),
`docs/specs/SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md` §8 (the 2026-07-04 incident
that was prematurely closed — see §6),
`docs/specs/SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md` (the reveal-gate/latch
machinery this spec touches),
`docs/specs/SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` §4.5 (the
duplicate-name `"X #N"` format implicated in §8's secondary finding).

---

## 1. Symptom

After the app entered an error state and the user clicked **recover**, tabs and pane
geometry were restored, but **five agent panes rendered as a completely blank region**.

Critically, this is *not* "a pane with no content". The pane **chrome is gone too** — no
header, no border, no title bar, no agent name. The tile still occupies its full rectangle,
so the layout reserves a large black void where five panes should be.

Distinguishing properties, all confirmed live:

- **Not transient.** Switching tabs back and forth many times does not repaint it.
- **Not data loss.** The `Layout` RPC still lists all 9 panes for that tab (5 of them
  `view: "agent"`); the underlying `claude.exe` processes are alive; transcripts read back
  intact via `GetAgentTranscript`.
- **No spinner, no error card.** Neither `block.tsx:489`'s `BrainSpinner` nor
  `BlockErrorBoundary.tsx` appears — see §4, this is load-bearing negative evidence.

## 2. Direct evidence

**2.1 — Both tabs are mounted, and the layout tree is intact.** `Layout(query:"layout")`
returns two tabs in one window. Tab `cec3b2d4-59f2-45b3-94f2-79dc8af030c6` lists 9 panes,
including `25cff0d0…` (Clamk), `2121195c…` (Agent1), `a5ecc604…` (Agent3),
`2942426d…` (Agent2), `ef7632bd…` (AgentY) — all `view: "agent"`. Those are exactly the
five that render nothing. The backend's view of the layout is **not** corrupted.

**2.2 — The tab↔block association flipped at recovery.** The same five blocks resynced
under their correct tab before the incident and under a *different* tab after it
(`agentmux-launcher.log`, srv 27512):

| Block | Agent | Before | After (18:29:15) |
|---|---|---|---|
| `25cff0d0…` | Clamk | `tab_id=cec3b2d4…` (09-19 08:36:34) | `tab_id=5e71cdeb…` |
| `a5ecc604…` | Agent3 | `tab_id=cec3b2d4…` (09-20 04:42:34) | `tab_id=5e71cdeb…` |
| `ef7632bd…` | AgentY | `tab_id=cec3b2d4…` (09-20 08:13:39) | `tab_id=5e71cdeb…` |
| `2121195c…` | Agent1 | — | `tab_id=5e71cdeb…` |
| `2942426d…` | Agent2 | — | `tab_id=5e71cdeb…` |

Every `ControllerResync` in the recovery burst carried the *same* `tab_id`
(`5e71cdeb…`, the window's static tab), regardless of which tab each block actually
belongs to.

**2.3 — Corroborating UI corruption.** Both tabs are now named `"Tab 2"`. The user
confirms they were `"Tab 1"` and `"Tab 2"` before the crash. Tab identity itself was
overwritten, consistent with §3.2.

## 3. Root cause

Two distinct defects compose. §3.1 is why the pane renders nothing *and stays that way*;
§3.2 is the trigger that puts a leaf into that state.

### 3.1 The hoist deadlock (render side — the reason it is blank and permanent)

**Space and content are decided in two different files, and only one of them can fail.**

- `frontend/layout/lib/TileLayout.core.tsx:581-593` — the `.tile-node` carries the
  geometry transform **unconditionally**. `:521-525` wraps whatever `renderContent`
  returns in a `.tile-leaf`. The tile holds its rectangle whether or not anything renders
  into it. *This is the code that "determines the empty pane space".*
- `frontend/app/tab/tabcontent.tsx:79-81` — `renderContent` → `<PaneLeafChrome/>`.
- `frontend/app/tab/pane-leaf-chrome.tsx:419-430` — the branch that yields nothing:

```tsx
<Show when={hoisted()} fallback={content}>
    <Show when={chromeVm()}>            // ← no fallback
        {(vm) => vm().renderPaneChrome!(chromeNodeModel(), content)}
    </Show>
</Show>
```

Both guards are **latched sticky**:

- `hoisted()` (`pane-leaf-chrome.tsx:134-140`) flips `true` permanently the first time
  `effectiveViewType()` is in `HOISTS_OWN_CHROME` (i.e. `"agent"`).
- `chromeVm()` (`:411-417`) caches into `latchedChromeVm`.

The deadlock: once `hoisted()` is true, `content` — the `<Block>` — is reachable **only**
as an argument to `renderPaneChrome`, which requires a non-null `chromeVm()`. But
`chromeVm()` reads `activeViewModel()`, which is published **only from inside a mounted
`<Block>`** (`block.tsx:352`). Block cannot mount without the ViewModel; the ViewModel
cannot exist without a mounted Block. The inner `<Show>` has **no fallback**, so the leaf
renders literally nothing — no chrome, no header, no border — while the tile keeps its
full geometry.

**This file already documents this exact failure occurring once before**
(`pane-leaf-chrome.tsx:297-317`):

> *"Reproduced live: `hoisted`/`keepAlive` both true, `stackBlockIds` correctly containing
> the blockId, yet the pane rendered **permanently blank** — not a data/layout-corruption
> issue (the block's own data was fully intact), a pure ordering race in this file."*

The eager `viewModelSlotFor(activeBlockId())` call added at `:320` closed the variant where
`viewModelSlots.get(id)?.get()` short-circuits on a not-yet-created slot. **The catch-22
above is the remaining variant and is not covered by that fix.**

**Why tab-switching never heals it.** The outcome is deterministic once block data is warm.
After recovery re-hydrates MOS, `effectiveViewType()` resolves to `"agent"` synchronously,
so `hoisted()` is already true on the *first* render — the `fallback={content}` branch (the
only path that ever mounts the first `<Block>`) never runs. Every subsequent remount
reproduces the identical state. This matches the observed "switching tabs many times does
nothing" exactly.

### 3.2 Wrong-tab controller registration (the trigger)

`frontend/app/store/window-identity.ts:19` — `staticTabId` is *"set once at init, never
change"*. Line 51-53 carries an explicit warning that it is the wrong value for backend
calls:

> `// NOTE: uiContext must use activeTabId (derived from workspace), NOT staticTabId.`
> `// staticTabId is set once at init and never changes. activeTabId tracks the`
> `// workspace's current active tab so backend service calls get the correct tab.`

Nonetheless, the controller-resync call sites send exactly that —
`useAgentControllerStatus.ts:568`:

```ts
await RpcApi.ControllerResyncCommand(TabRpcClient, {
    tabid: staticTabId(),      // ← window-level, not this pane's own tab
    blockid: opts.blockId,
```

Because **every tab stays mounted** (stated at `layoutPersistence.ts:118`), all agent panes
across *all* tabs re-run this on recovery, and each reports itself as belonging to the
static tab. That is precisely the flip captured in §2.2.

`quick-fork.ts:117-128` already documents this hazard class and works around it locally by
capturing `atoms.activeTabId()` synchronously:

> *"`ControllerResyncCommand` uses `atoms.staticTabId()` (fixed at window bootstrap, not
> necessarily this tab) when no override is given, and would silently register the new
> block under the WRONG tab (Codex's review of this PR)."*

That workaround was applied to exactly one call site. The rest were not.

**Note on correctness:** `activeTabId` is *also* not universally right here. For a pane in
a background tab, neither `staticTabId` nor `activeTabId` is correct — the only correct
value is **the block's own tab**. §8.2 specifies this.

### 3.3 How they compose (likely, not yet proven end-to-end)

Proposed chain: wrong-tab registration causes the block's `<Block>` component to mount
under the other tab's registry (`block-component-registry.ts:29-38`;
`getLayoutModelForStaticTab()` resolves only the active tab's tree), so the background
tab's leaf never observes an `activeViewModel` → `chromeVm()` latches null → §3.1's
deadlock → permanent void.

`layoutPersistence.ts:113-120` independently describes this composition:

> *"A dangling leaf renders a block owned by ANOTHER tab; since every tab stays mounted,
> the same block mounts twice and the block-component registry breaks — observed as a
> fully non-responsive tab."*

**Confidence:** §3.1 and §3.2 are each independently proven (code + logs). The causal link
between them in §3.3 is strongly supported but not yet reproduced in isolation. §9.1
specifies the decisive diagnostic that would confirm or refute it. **The §8 fixes do not
depend on §3.3 being correct** — each layer is independently justified.

## 4. Negative evidence (why every "missing data" explanation is ruled out)

The absence of these is diagnostic, not incidental:

- `block.tsx:378` + `:473-500` — `<Show when={ready()}>` has no fallback; a not-ready block
  shows a `BrainSpinner` overlay (`:489`). **No spinner appeared.**
- `BlockErrorBoundary.tsx` — per-pane error UI for thrown render errors. **No error card
  appeared.**
- `tabcontent.tsx:164` — `"Tab Not Found"` is tab-level only, and did not appear.
- `block.tsx:120-122` — `BlockPreview` returns null, but that is preview-only.

If `<Block>` had mounted and merely lacked data, one of the first two would be on screen.
Neither is. **Therefore `<Block>` was never mounted at all** — which eliminates every
block-data branch and isolates the failure to `pane-leaf-chrome.tsx:419-430`.

## 5. Why the existing self-healing pass cannot catch this

`pruneDanglingLeaves` (`layoutPersistence.ts:135-176`) is the designated healer for dead
space. It fails here for two independent reasons:

1. **Wrong predicate.** It prunes leaves whose `blockId` is not in `tab.blockids`. Here the
   backend still correctly owns all five blocks in the affected tab (§2.1), so they are not
   "dangling" by its definition. The divergence is in *controller registration*, not tab
   ownership — a dimension this predicate does not model.
2. **Not reactive.** Triggers are only: model init +2s (`:53`), the tab bar's post-drag
   settle, and the redock failure path (`:131-133`), with an explicit
   `NOTE deliberately NO prune here` at `:76`. Nothing re-runs it on tab switch, which is
   why the user's repeated tab-switching produced no change.

## 6. This is the third occurrence of this symptom class

| Date | Event | Outcome |
|---|---|---|
| 2026-07-04 | `sysinfo`/`swarm` panes pruned from a live tree, *"leaving a stale-rendered empty cell"* | Closed in `SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md` §8 as *"very likely already fixed"* — reasoned, not proven |
| 2026-07-08 | Dead-space leaf referencing a deleted block | `INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION` — Mechanism A fixed; **systemic enabler left open** |
| 2026-09-20 | **This incident** — dead space via hoist deadlock + wrong-tab registration | New mechanism; enabler still open (§7) |

The 2026-07-08 investigation named its own falsification condition, which this incident
satisfies:

> *"If this class of symptom (a pane's content gone, empty cell persists) recurs on the
> current (post-864) build, that would be strong evidence against this conclusion and
> should be treated as a fresh, still-open bug."*

It also flagged that the named regression guard does not cover this class:
`layout_stays_coherent_across_full_mutation_lifecycle` (`agentmux-srv/src/server/tests.rs`)
asserts only `TabRecord.rootnode == db_layout.rootnode` after serial, single-actor reducer
dispatches — **no frontend `LayoutModel` in the loop, no concurrent/stale-push simulation,
no dangling-reference assertion.** That remains true today.

## 7. The systemic enabler is still open (re-verified on `main` @ `1e976aac8`)

Every write-path gap identified on 2026-07-08 is unchanged two and a half months later:

| Gap | Location (verified today) | Status |
|---|---|---|
| `Store::update` documents "optimistic locking" but has no `WHERE version = ?` | `agentmux-srv/src/backend/storage/store.rs:488, 510, 710` | ❌ blind overwrite + blind version increment |
| `handle_layout_set_tree` assigns unconditionally | `agentmux-srv/src/reducer/layout.rs:105` (`tab.rootnode = new_tree.clone();`) | ❌ no comparison against prior tree |
| Persist has no referential-integrity check | `agentmux-srv/src/persist_subscriber.rs` (`apply_layout_tree_replaced`) | ❌ per the 07-08 investigation |

Consequence, quoting that investigation: *"any full-tree push that reaches the reducer
after a delete, no matter how stale, silently wins and gets persisted. Nothing in the stack
can reject it."*

## 8. Proposed fixes

Three independent layers. **Layer A alone converts this from a silent permanent void into
a visible, recoverable state** and should ship first even if B and C are deferred.

### 8.1 Layer A — the render safety net (highest priority, smallest diff)

`pane-leaf-chrome.tsx:419-430`: the inner `<Show when={chromeVm()}>` must have a fallback.
A leaf must never be able to render nothing.

Acceptance criteria:

- With `hoisted() === true` and `chromeVm() === null`, the leaf renders a visible
  placeholder (spinner or error card), **never** an empty `.tile-leaf`.
- The placeholder must offer a user-visible recovery affordance (reload/reopen this pane),
  since §3.1's latches make the state unrecoverable by tab-switching.
- Must not reintroduce the chrome-remount flash that the latches at `:128-140` exist to
  prevent (`SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md`).
- Preferred: break the catch-22 itself — ensure a `<Block>` is mounted (hidden if needed)
  so the ViewModel can be published even on the hoisted path, rather than only papering
  over the symptom with a fallback.

### 8.2 Layer B — tab-id correctness at the resync call sites

All of these send `tabid: staticTabId()` unconditionally and should send **the block's own
tab id**:

`useAgentControllerStatus.ts:568`, `agent-model.ts:314`, `agent-model.ts:744` (has an
override param already), `runtime-apply.ts:99`, `launch-flow.ts:367`,
`AgentShellSubblock.tsx:466`, `termViewModel.ts:586`, `termwrap.ts:852`,
`termosc.ts:274,286`.

Acceptance criteria:

- A resync issued from a pane in a **background** tab registers under that pane's own tab,
  not the active or static tab. `activeTabId()` is insufficient — see §3.2's note.
- Regression test: mount panes in two tabs, trigger a resync from the background tab,
  assert the recorded `tab_id` matches the pane's own tab.
- Consider making the wrong thing hard to do: have the RPC wrapper derive the tab from
  `blockid` server-side, or require an explicit tab argument with no `staticTabId` default.

### 8.3 Layer C — close the systemic enabler (§7)

- Real CAS on the layout write path: carry an expected-prior-version on
  `Command::LayoutSetTree`; `Store::update` must use `WHERE version = ?` and reject on
  mismatch rather than blind-overwrite (`store.rs:488,510,710`).
- Staleness check in `handle_layout_set_tree` (`reducer/layout.rs:105`).
- Referential-integrity validation at persist time: reject a tree whose leaves reference
  unknown blocks.

### 8.4 Regression guard that actually covers this class

The existing guard provably does not (§6). A new test must:

- Drive a real frontend `LayoutModel` (not just reducer dispatches).
- Simulate two mounted tabs with a cross-tab registration divergence.
- Assert **no leaf can ever render zero elements while holding non-zero geometry** — this
  is the invariant whose absence allowed all three occurrences.

## 9. Open questions

**9.1 — Decisive diagnostic, not yet run.** Inspect a blank region in DevTools:

- `.tile-node` with `visibility:hidden; opacity:0` ⇒ stuck reveal gate
  (`TileLayout.core.tsx:573-579`, gate state in `frontend/app/store/tab-reveal.ts:252`,
  released only via rAF-driven `scheduleOnSettle`). Module-global and **not tab-scoped**.
- Normal `.tile-node` containing an **empty `.tile-leaf`** ⇒ the §3.1 hoist deadlock.

The reveal-gate path is considered less likely (held only by `quick-fork.ts:135` and
`open-history-tab.ts:90` — an unlikely source for five panes at once), but this single
observation discriminates definitively and should be recorded here before implementation
begins.

**9.2 — Tab name overwrite (§2.3) is unexplained.** Both tabs ending up named `"Tab 2"` is
consistent with tab-identity confusion but its specific write path has not been traced.

**9.3 — Is `stackMembers.ts` involved?** Assessed and ruled out: it is pure, mutates
`TabLayoutData` in place, returns `false` without changing anything on invalid input
(`:28`, `:69`, `:105`), refuses to remove below `length <= 1`, and `effectiveStack`
(`:13-15`) falls back to `[data.blockId]`. Only degenerate case is a blank `blockId`.

## 10. Secondary finding (separate bug, filed here for traceability)

Two agents in this incident (`Naki #2`, `Loap #2`) were additionally **unreachable over
muxbus**, independent of the rendering failure. Their agent ids fail validation:

- `agentmux-srv/src/backend/reactive/sanitize.rs:124` — `validate_agent_id` permits only
  `[A-Za-z0-9_-]`.
- `agentmux-srv/src/server/agent_handlers/template.rs:84` — the fork/duplicate naming rule
  generates `format!("{root_name} #{}", …)`, i.e. a space and `#`, per
  `SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` §4.5.

Every such agent therefore fails `muxbus: persistent auto-register` deterministically, on
every registration attempt, forever (observed in logs on 09-15 and again on 09-20). Per
`SPEC_AGENT_NAMING_AND_ADDRESSING…` line 330, the routing identity is meant to be tied to
the definition **slug**, not the display name — so the fix is to register the slug and let
the display name stay human-friendly. Recommend a dedicated spec/PR; it is out of scope
here.
