# Spec: verbosity modes for the agent pane (Full / Standard / Replies)

Author: Opaz@charlie. Date: 2026-10-29. Kanban #10718 (research half: #10717, published as the docs page "What the agent pane shows", agentmux-docs #225, merged 32b888ae). **Status:** In progress. Approved by the operator; built in phases, the first (the view, hidden-run lines, `Alt+V` and the palette commands) in this PR, the View popover and Settings section next.

Code read at `origin/main` 458192d05. Other-UI research: `scratch/taxonomy/part-e-other-uis.md` (URLs per claim).

## 0. Summary and recommendation

Revision 2 (same day): the operator asked for per-type control, not only presets ("an option to keep command calls collapsed, and jekt calls collapsed"). Presets stay, and **per-type switches are in the first release**.

Today the agent pane shows every row the agent produces, and there is no way to show less. The only per-row control is folding a row (and pinning it open). A "filter" record (`showThinking`, `showSuccessfulTools`, `showFailedTools`, `showIncoming`, `showOutgoing`) is saved with each pane but **nothing reads it**.

Recommendation, in one paragraph:

- **Five switches**, one per kind of routine row: **Thinking**, **Tool calls** (commands, file reads and edits, searches, shells), **Jekts and agent messages**, **AgentMux notes**, **Context and setup**. Each is *Auto / Expanded / Collapsed / Hidden* (notes: Shown / Hidden).
- **Three presets** that set the switches: **Full** (everything as today), **Standard** (thinking, tool calls and jekts collapsed to one line, setup hidden), **Replies** (only the conversation; the rest hidden). Change one switch and the label reads **Custom**.
- It is a **view-layer filter only**. The rows, the history, the agent's context, search, copy and export stay complete. Hidden rows sit in "N hidden" chips you can open in place. Switching re-derives the list with no reload.
- A **global default** plus a **per-pane override**, set from one popover in the pane menu and the composer strip, plus a command palette entry and `Alt+V` to cycle the presets.
- A **never-hide list that no switch can override**: errors, failure rows, anything waiting for you, questions, your own messages, sensitive jekts.
- A live "Working" line so a pane with hidden rows never looks stuck.
- Default stays **Full** in the first release; the operator flips it after trying it.

Decisions are numbered in section 6: five that need the operator's choice come first, the others are "as recommended unless you object".

## 1. The request (the operator's words, relayed by AgentY, 2026-10-29)

> lets also design a verbose mode, or rather different modes. i think right now we are in verbose, right, in the agent pane, we get everything? We would want to be able to get different shorter versions, perhaps one with only the explainer text (what is that called? there is also the thinking text that is italics. Commands and their collapsable previews, jekt messags, and the agentmux notes, we want a full taxonomy of this, also get it into an articale in agentmux-docs (unless it already exists) .. so we know what every message type in an agent pane is. Then we'd work on best practice verbose controls

Follow-up (2026-10-29, relayed by AgentY): "how are we doing with the verbose options? one thing I notice is an option to keep command calls collapsed, and jekt calls collapsed." So per-type control matters as much as presets; AgentY's direction: keep the three presets and add per-type switches in the first release, picking a preset sets the switches, changing one shows "Custom", same global default plus per-pane override, never-hide list absolute.

Answers to the questions in it:
- "The explainer text" is a **Reply** (code: a `markdown` node, "assistant text"). The italic text is **Thinking** (a `markdown` node, thinking). Both are named in the published taxonomy.
- Yes: we are in "Full". Every node type is shown; thinking is fixed open; the only reduction is that finished tool rows close themselves.
- The taxonomy exists now: docs page `/agent-pane-messages/` (#225). This spec uses its names.

## 2. Today, from the code

- Rows are the `DocumentNode` union (`frontend/app/view/agent/types.ts`): `markdown`, `tool`, `agent_message`, `jekt_message`, `system_note`, `user_message`, `shell`, `agent_error`, `context_compacted`, `compaction_started`, `memory_reinjection`, `context_delivery`, `session_outcome`, `cli_notice`, `day_divider`, `history_link`, `resume_preflight`, `ambient_narration`. Failure rows, banners and panels are not nodes; they sit around the list.
- Open or closed is decided in one function, `rowDisclosure` (`virtualization/disclosure.ts`): fixed, open/collapsible, closed/pinnable, plus the "held open while on screen" rule. Fold and pin state (`collapsedNodes`, `pinnedNodes`, `heldOpenNodes`) is saved per pane.
- The list is virtualised (`AgentDocumentVirtualList`, heights from `estimateNode` in `renderers.ts`). The live feed keeps about 1 MB of turns (`agent:livefeed`); older turns are in the History tab, which has its own default filter.
- `FilterState` is persisted by `useSnapshotPersistence` and read by nothing. So there is **no existing user-facing control to extend**, and no behaviour to keep compatible.
- Providers differ: Gemini CLI and Antigravity send no thinking; Aider and Crush are raw terminals with no rows at all. A mode must degrade gracefully there (section 4.9).

## 3. Best practice from other agent UIs

(Details and URLs: `part-e-other-uis.md`.)

- **Most products converge on three tiers**: quiet (final answer plus a tool summary), default (tool one-liners, thinking collapsed), full (thinking plus complete tool input and output). Claude Code: default / verbose / focus view. Cursor: Compact / Balanced / Detailed. Gemini: compact tool output and inline thinking as two settings.
- **Thinking and tool output are separate axes** in Zed, Copilot and Gemini; users ask for the split in the products that tie them together. A single preset that sets both, with the axes kept separate in the data, is the common answer. (We do presets only in v1; section 6, D1.)
- **Controls live in a global setting plus a quick toggle**: settings file for the default, a key or slash command for the moment (Claude Code `Ctrl+O`, `/focus`; opencode `/details`, `/thinking`; Gemini `Ctrl+O`).
- **Failure modes to design against**:
  - Hidden thinking reads as "stuck" (Claude Code #24285): keep a live working indicator with elapsed time.
  - One-liners without the object hide what changed ("Read 3 files", Claude Code #22458, Copilot #292985): every fold names its target.
  - Approvals buried inside collapsed groups (Cursor MCP Allow/Deny, users think the agent hung): anything waiting for the user is outside every fold.
  - Expanding or collapsing jumps the layout (VS Code #325549): anchor on the row the user is looking at.
  - Settings that silently don't take effect (Codex #7090, Zed #62145, Gemini #24583): one code path, covered by a test that every node type has a rule.
  - Over-correcting to hide-by-default (Codex v0.24.0): do not change the default in the first release.
- **Model-side verbosity is a different thing** (Codex `model_verbosity`, Claude Code output styles). This spec never changes what the model writes or what the agent sees.

## 4. Proposal

### 4.1 The switches and the presets

The operator's example ("keep command calls collapsed, jekt calls collapsed") is a per-type control. So the unit is a **switch per kind of row**, and the modes are named sets of switch values.

| Switch | Rows it covers (published names) | Values |
|---|---|---|
| **Thinking** | Thinking, Canceled thought | Auto, Expanded, Collapsed, Hidden |
| **Tool calls** | Tool rows (commands and their previews, reads, edits, searches, MCP), Shell | Auto, Expanded, Collapsed, Hidden |
| **Jekts and agent messages** | Jekt, Agent message | Auto, Expanded, Collapsed, Hidden |
| **AgentMux notes** | System note, Ambient line | Shown, Hidden |
| **Context and setup** | Startup context, Compacting, Memory reinjected, Context delivered, CLI notices that are information or success | Auto, Hidden |

What the values mean:
- **Auto**: the row's own rule today (`rowDisclosure`): a tool is open while it runs, held open if it finishes on screen, then closed; a jekt is closed and held open if it arrives live; thinking is open. This is what "Full" has.
- **Expanded**: open by default; you can still fold it with a click or `e`. A long feed of expanded tool bodies costs more to render; each body stays capped (about one sixth of the window height).
- **Collapsed**: a one-line fold that names its target (the command, the path, the sender) and shows its status and elapsed time; it is **never held open** when it finishes, and a running tool shows its live tail on the line. Click or `e` opens it; pin keeps it open.
- **Hidden**: not in the list; counted in the "N hidden" chip (4.3). Pinned rows and the never-hide list (4.4) are exempt.

Rows that have no switch are always shown, as today: your messages, broadcasts you sent, Supervisor nudges, **Replies**, Question and answer, Errors, Context compacted, Session outcome, warning and error CLI notices, Resume check, History link, Day divider, Loading row. They are the conversation and the things that explain it.

**Presets** (the values each one sets):

| Switch | Full | Standard | Replies |
|---|---|---|---|
| Thinking | Auto | Collapsed | Hidden |
| Tool calls | Auto | Collapsed | Hidden |
| Jekts and agent messages | Auto | Collapsed | Hidden |
| AgentMux notes | Shown | Shown | Hidden |
| Context and setup | Auto | Hidden | Hidden |

- **Full** equals today exactly (a snapshot test, 4.8), so shipping this changes nothing until someone chooses.
- Picking a preset sets all five switches. Changing any one switch to a value that does not match a preset shows the label **Custom** (the label is derived: the current five values are compared with the three presets, so there is nothing to keep in sync).
- Hand-set values are kept: Custom is "Standard, but Tool calls expanded" or "Replies, but Jekts collapsed", whatever the user built.

### 4.2 Rules that apply on top of the switches

- **A tool row awaiting approval or an answer is Always shown**, whatever the Tool calls switch says, even though its panel is also above the composer.
- **When a forced-visible row resolves** (you approved it, a pending question ended, a sensitive jekt was read) it stays as a one-line fold until it scrolls off the top, then follows its switch. This reuses the held-open rule (`heldOpenNodes`): nothing disappears under your eyes.
- **Your own fold and pin choices win.** A pinned row stays visible in any mode, even under Hidden. A row you folded or opened by hand keeps that state until you change the switch for its type; changing a switch clears your hand-made folds for that type's rows (pins are kept), so "collapse all tool calls" does what it says.
- **Failed tool calls**: under Hidden they are inside the chip, counted `✗ N` in the warning colour (D9).
- A **jekt from a person's device** or a **sensitive jekt** is Always shown (4.4); under Collapsed it is a fold, under Hidden it is not hidden.
- **Providers**: a switch for something the provider never sends (thinking on Gemini CLI or Antigravity) does nothing; for a raw terminal agent (Aider, Crush) the control is disabled (4.9).

### 4.3 The hidden-run chip

Consecutive hidden rows between two visible rows become **one chip**: `▸ 12 tool calls, 3 thoughts, 1 jekt` (counts by canonical kind; a `✗ 2 failed` part in the warning colour if any failed). Click it (or `e`) to open the run **in place** with the rows shown in their Full form; the header stays so you can fold the run again. Open/closed state of a chip is saved with the pane like other fold state, keyed by its first hidden row so it survives new rows being appended.

While the agent is busy and the newest rows are hidden, the chip at the bottom is also the **live line**: `▸ Working · Bash cargo test · 14 s · 12 hidden` (or `Thinking · 8 s` when only thinking is happening). This answers "is it stuck?" in Replies, and is the same text in Standard where a thought is folded. No new timers: it derives from the running tool row and the stream's busy state.

### 4.4 Never hide

With any combination of switch values, and not reachable by any of them:
- Error rows and failure rows (and their buttons).
- Any tool row awaiting approval or an answer, and the Q&A pair after it.
- Your messages, broadcasts you sent and Supervisor nudges (they are what instructed the agent).
- A **sensitive** jekt, and a jekt from a person's device (that is the operator's voice arriving by another route).
- The cloud "expired undelivered" note (something was lost).
- Context compacted and Session outcome (they explain why earlier context is missing or why this is a fresh start).
- Everything that is not a row (panels, dock, banners).

A switch can only hide **routine work that completed or is plumbing**. A test enumerates this list (4.8).

### 4.5 Where the choice lives

- **Data**: five values (`thinking`, `tools`, `jekts`, `notes`, `context`). A *view* is just that set of five; the preset name is derived from it.
- **Global default**: setting `agent:view` in the settings file, a small object `{ "thinking": "auto", "tools": "auto", "jekts": "auto", "notes": "shown", "context": "auto" }`. A string shorthand is accepted and expanded: `"full"`, `"standard"`, `"replies"`. Default `"full"` in the first release (D2). Added to `schema/settings.json` with the other `agent:` keys, with a section in Settings (the preset plus the five rows).
- **Per-pane override**: the same object in the pane's own metadata, saved with the pane and visible to the Swarm. A pane either **follows the default** (nothing stored) or has **its own complete view**: there is no per-key merging with the global one, so what a pane shows is never a surprise mix. Changing a switch in a pane that follows the default copies the effective view into the pane and changes that one value.
- **The control**: a **View** popover from the pane menu and from a small chip at the right of the composer strip. It has the preset buttons (**Full / Standard / Replies**, with **Custom** lit when the five values match none), the five switches as small segmented controls (Auto / Expanded / Collapsed / Hidden), a **Follow default** toggle, and **Make this the default** (writes the global setting). The chip shows the preset name, plus `· this pane` when the pane overrides, and is hidden while the pane follows a default of Full.
- **Command palette**: **Agent view…** opens the popover; **Agent view: Full / Standard / Replies / Follow default** set the pane directly. No per-switch palette entries (the popover is two clicks).
- **Keyboard**: `Alt+V` cycles Full, Standard, Replies for the focused agent pane (a Custom pane goes to Full first); `Alt+Shift+V` clears the override. `Alt+V` is unused in the shortcut list (the nearest are `Alt+K` Mention in agent and `Ctrl+Shift+V` paste). On macOS `⌥V` types a character in a text field; the palette entry is the fallback if it conflicts. To be confirmed in the build (D11). `Ctrl+O`, used by Claude Code for its transcript, is deliberately not taken. Individual switches have no key in the first release (the popover and the palette entry are enough; D11).
- **Remove the dead `filter` record** from the snapshot and `DocumentState`: the per-pane value is in pane meta, the global one in settings. Old snapshots with a `filter` are read and ignored (nothing read it before).

### 4.6 What happens to history already loaded

It is the same list, derived again. The display list is a pure function of (nodes, view, fold and pin state, held-open set); changing the view recomputes it and keeps the same nodes, so there is **no reload, nothing is lost and switching back restores exactly what you had**.

- **Scroll anchoring** (the VS Code #325549 problem): before the switch, remember the topmost visible row (or the chip that now contains it) and its offset from the top of the pane, and restore it after. If the pane was following the bottom, it keeps following.
- Fold and pin state is per node. Picking a **preset** or changing **one switch** clears the hand-made folds of the rows that switch governs (pins are kept); other rows keep theirs. Chip open state is per run.
- **Agent History tab**: uses the same selector but starts in Full regardless of the pane's mode (it is the forensic view; D6), with its own toggle.
- **Search** (`Ctrl+F`): searches every row, hidden or not. A match inside a chip opens that chip until the search bar closes. **Copy and export** (`agent:transcriptsinglecopy`, transcript copy) always include every row. The hover preview's token estimates are unchanged.
- The agent's context, the saved history and the mobile app's feed are untouched.

### 4.7 Implementation sketch (for the build task)

- New pure module `frontend/app/view/agent/virtualization/verbosity.ts` next to `disclosure.ts`:
  - `VIEW_PRESETS` (the three rows of the preset table) and `presetOf(view)` (derives the label, or `custom`).
  - `SWITCH_OF_NODE`: for each node kind (and the sub-kinds that matter: thinking, canceled, sensitive jekt, person's-device jekt, CLI notice tone) which switch governs it, or `none`. Typed so a new `DocumentNode` kind is a compile error until it has an entry (the idea of `disclosure.parity.test.tsx`).
  - `deriveDisplayNodes(nodes, view, state)`: applies the switch value to each node (Collapsed and Expanded feed `rowDisclosure`; Hidden removes it), and returns the visible nodes plus synthetic `hidden_run` nodes (id from the first hidden node), counts by kind, and a `live` flag when the tail is busy.
- `rowDisclosure` takes the switch value as an input, so Collapsed (closed, never held open) and Expanded are parameters, not a copy of the rules.
- `AgentDocumentVirtualList` consumes the derived list; `estimateNode` gets one case for `hidden_run` (one line).
- Settings and pane meta read through the existing agent-setting hooks; the popover is a small component beside the composer strip; the keyboard binding and palette commands are registered with the other agent-pane commands.
- Full must produce the same display list as today (a snapshot test over a recorded transcript).
- Less rendering in Replies and Standard is a side effect: tool bodies (highlighted code, diffs) are the expensive rows, and Collapsed or Hidden never mounts them unless opened. Measure with `scripts/perf/memory-lab` before claiming it; no change may trade performance for memory (standing rule).

### 4.8 Tests

- Switch map is exhaustive over `DocumentNode` kinds (type-level plus a runtime parity test).
- Never-hide list: for every combination of switch values, a fixture containing every Always row yields all of them visible.
- Full == today: the derived list equals the current list for a recorded transcript with all kinds.
- Presets: each preset sets the five switches; any single change shows Custom; setting them back to a preset's values shows that preset.
- Resolution: a pane that follows the default, a pane with its own view, "Make this the default", and a Custom pane.
- Changing a switch clears hand-made folds for that type and keeps pins; pinned rows stay under Hidden.
- Chip counts and run keys stay stable when rows are appended, and when a hidden row resolves from pending (held-open).
- Scroll anchoring across a switch change with a following and a non-following pane.
- Search reveals a hidden match and restores the chip afterwards.
- Visual: before/after screenshots in the three presets and one Custom view on the same transcript (the operator reviews by dogfooding; send screenshots to AgentY).

### 4.9 Providers

- **Gemini CLI, Antigravity** send no thinking: the Thinking switch does nothing. Nothing special.
- **Aider, Crush** are a raw terminal with no rows: the View control is disabled in those panes (a tooltip says why), and the setting has no effect.
- **Codex** shows its reasoning and plan updates as Thinking, so they follow the Thinking rules. **Muse** sends reasoning only when complete: still Thinking.
- Container agents and agents on other hosts are the same: it is purely frontend presentation, so no host-exec reason to restrict it.

## 5. What this does not do (and could later)

- No model-side change (output style, `model_verbosity`).
- No per-agent default stored in the agent's definition; per-pane and global only (D3).
- No per-switch keyboard keys and no per-switch palette entries in the first release.
- Reply, Q&A and the rows in the always-shown list have no switch by design.
- The mobile app's feed is out of scope; a view there is a later mobile spec if wanted.
- Docs: the "What you can hide today" section of the agent-pane page (#225) gets rewritten in the build PR, so docs go live with the release.

## 6. Decisions and recommendations

### The five that matter (please decide these)

| # | Decision | Recommendation |
|---|---|---|
| D1 | **Presets, per-type switches, or both** (4.1) | **Both, in the first release.** Five switches (Thinking, Tool calls, Jekts and agent messages, AgentMux notes, Context and setup), each Auto / Expanded / Collapsed / Hidden (notes: Shown / Hidden). Full / Standard / Replies set them; changing one shows Custom |
| D2 | **Default view** | **Full** in the first release; the operator flips it to Standard after dogfooding. Matches the "don't over-correct" lesson (Codex hid thinking by default and users pushed back) |
| D4 | **The never-hide list** (4.4), absolute | Errors and failure rows; anything awaiting approval or an answer, and the Q&A after it; your own messages, broadcasts and Supervisor nudges; sensitive jekts and jekts from a person's device; the "expired undelivered" note; Context compacted and Session outcome. **No switch value can hide these.** Everything else is routine and can hide |
| D3 | **Where the choice lives** (4.5) | Global `agent:view` setting plus a per-pane override in pane meta. A pane follows the default or has its own complete view (no per-key merging). One View popover (pane menu and composer chip); `Alt+V` cycles presets; palette entries as fallback. No per-agent stored default |
| D6 | **History already loaded** (4.6) | A view-layer re-derive of the same rows: no reload, nothing lost, switching back restores everything; scroll anchored on the row you are looking at; changing a switch clears hand-made folds of that type but keeps pins; Agent History tab starts in Full; search reveals hidden matches; copy and export always include everything |

### The rest: as recommended unless you object

| # | Decision | Recommendation |
|---|---|---|
| D5 | Thinking in Standard | **Collapsed to one line with its duration**, not hidden: shows the agent is alive, costs one line |
| D7 | Tool calls in Standard | **Collapsed one-liners that name the target**; the running one shows a live tail; never held open |
| D8 | The hidden-run chip (4.3) | One chip per run of hidden rows between visible rows, counts by canonical kind, `✗ N` in the warning colour if any failed; click opens the run in place; at the bottom it doubles as the live "Working · Bash cargo test · 14 s" line |
| D9 | Failed tool calls under Hidden | **Inside the chip, counted `✗ N`**. Errors that matter reach the reply or become a failure row |
| D10 | Jekts and agent messages | **Collapsed in Standard, Hidden in Replies**, except sensitive jekts and jekts from a person's device (Always shown) |
| D11 | Keyboard | `Alt+V` cycles presets, `Alt+Shift+V` follows the default; no per-switch keys; confirm `⌥V` on macOS in the build, palette entries as fallback |
| D12 | Pinned rows vs the switches | **Pinned wins**, even under Hidden |
| D13 | The unused `filter` record | **Remove it**; replaced by pane meta plus the setting |
| D14 | System notes in Replies | **Hidden**, except "expired undelivered"; the paused and resumed notes become chips |
| D15 | Names | **Full / Standard / Replies**; the five switches as named in 4.1; values **Auto / Expanded / Collapsed / Hidden** |

## 7. Phases

1. **Core**: `verbosity.ts` (presets, switch map, derivation), `rowDisclosure` input, chips, setting `agent:view`, pane meta, the View popover (pane menu and composer chip), palette entries, `Alt+V`; tests in 4.8. Default Full. One PR; frontend only.
2. **Live line and search**: the Working line, search reveal, History tab toggle, scroll-anchoring polish, screenshots in the three presets and a Custom view.
3. **Docs and default**: rewrite "What you can hide today" in the agent-pane page and add the setting to the settings page, with the release. The operator decides whether Standard becomes the default.

Estimated size: phase 1 is a medium PR (one new module, one list change, one setting, one popover); phases 2 and 3 are small.

No build starts until the operator approves this spec in conversation.
