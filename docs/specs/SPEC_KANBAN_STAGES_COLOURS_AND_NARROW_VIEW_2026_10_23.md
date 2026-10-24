# Spec: the Kanban's stages (Backlog, In Research, Spec'd, In progress, Review, Merged, Released), its colours, and its narrow view

**Status:** active — approved by the operator, 2026-10-23 ("looks good, kanban colors look good lets do it", relayed by AgentY), with D1–D7 as recommended (§13), plus the changes of §4.5 the same day. Built in Kanban #1708 (which replaces #1106).
**Date:** 2026-10-23
**Author:** Naki-2@narko, for Kanban tasks #1103 (this spec) and #1106 (its build), filed by AgentY at the operator's request
**Verified against:** `agentmux` `main` @ `9dfcbfac6` (2026-10-23), by reading code. Nothing here has been run.
**Changes:** `SPEC_WARDEN_KANBAN_2026_10_20.md` §6.3 (columns), §6.4 (who moves what), §6.7 (a card), §6.10 (attention), §6.11 (layout), §6.12 (how agents write), §6.15 (tasks that start after others).

## 1. Ask

The operator, 2026-10-23, in two messages relayed by AgentY:

> "We want it Backlog -> Spec'ed -> In progress, review, done .. the modus oprandi is I request a feature, a spec is to be written, then an implementation is a seperate phase."

> "I think we can change it from Backlog -> In Research -> Spec'd -> ... lets also add more colors so we can tell what is happening. use the agent colors (agents have colors) in the kanban, and each section should have color. Also, in the responsive thin view, have everything 1 long list with a single scrollbar, so it will overflow vertically"

So three things:
1. **Stages.** A feature goes request → research or spec → the operator approves it → build → review → done. The build is a separate task that starts only once the spec is approved. The board shows those stages as columns.
2. **Colours.** Each card shows its agent's colour. Each column has its own colour.
3. **Narrow view.** When the Kanban is narrow, it is one long list with a single scrollbar.

## 2. Summary

- **Eight columns:** Backlog, In Research, Spec'd, In progress, Needs you, Review, Merged, Released. Needs you stays. It is still the only column that interrupts the operator (§4.2). Review is the PR's review by muxreview, not the operator's; Done is now Merged; Released is merged work that has shipped (§4.5).
- **Every column is computed, none is stored.** It comes from the task's state, its `kind`, and the kinds and states of the tasks it starts after. There is no new task state or schema version, and older builds sharing the store see what they see today (§4).
- **A feature is two tasks:** "Spec: X" (kind `spec`, or `research`) and "Build: X" (kind `build`), with the build set to start after the spec. That is how features are filed today (#935/#936, #1103/#1106). This spec makes the pair one call (`Tasks` `enqueue` with `first: "spec"`), and a "Spec first" box in the operator's New task (§5).
- **Approval rules.**
  - **Accepting a spec moves its build into Spec'd** and tells the build's agent, because a task that becomes free doesn't wake an idle agent today.
  - **A spec or research task that a build waits on can't skip Review.**
  - **Cancelling a spec holds or cancels its build.** Today, cancelling a spec lets its build start (§6).
- **Colours.** A card has a stripe and chip in its agent's colour: the colour of the agent's pane and its Swarm row. Each column has a header bar in its own stage colour. All colours are theme tokens, and the words still carry the meaning (§7).
- **Narrow view (under 760 px of Warden width).** Everything is one list with one scrollbar. Each column is a section under its coloured, sticky header. Drag and drop still works (§8).

## 3. Today

- **Columns:** Backlog, In progress, Needs you, Review, Done (`frontend/app/view/warden-kanban/kanban-columns.ts`, `COLUMNS` and `columnOf`). A column is a pure function of the stored state and the holder's live state. `kind` plays no part.
- **Kinds:** a free tag (`WorkItem.kind`). In use: `spec`, `build`, `research` (e.g. #1255), and `review` and `repro` in the tool's examples. Most tasks have none.
- **"Starts after"** (`backend/storage/work_deps.rs`):
  - A task waits while any task it starts after isn't `done` or `cancelled`.
  - A `failed` one still holds it.
  - The card shows an hourglass pill "after #N".
  - `AfterTask` carries `id`, `num`, `title`, `state`, but not `kind`.
  - Nothing lists a task's followers.
- **Accept** (`server/app_api/kanban.rs` `act` `"accept"`) moves Review → done and records it. Nothing tells anyone that a follower is now free.
  - The next-task answer and the reminder would name it, but only when that agent next finishes a task or idles with reminders on.
  - The reminder is off by default (`SPEC_KANBAN_REMINDER_OFF_BY_DEFAULT_2026_10_22.md`).
- **Cancel** marks a task `cancelled`, which counts as finished, so its followers become claimable at once.
- **Review is skippable.** A task that doesn't need review (`review_required` false, the default for an agent-to-agent handoff) goes straight to done on `submit` or `complete`. A spec filed by one agent for another can therefore unlock its build with no person ever reading it.
- **Agent colours:**
  - **Stored:** each agent definition has one, as `#rrggbb`, in its content store (`ui:color`, `backend/agent_color.rs`, keyed by the definition id, which is the agent's uid).
  - **Pane:** when a pane opens, the colour is copied into its block meta (`frame:activebordercolor`). The pane header's Pane Color menu can override it with `frame:hue`.
  - **Swarm and Tower** read it from the block meta through `blockRoleColor` (`frontend/app/block/pane-identity.ts`). That maps it to theme-tuned OKLCH roles (`pane-color-scheme.ts`).
  - **Kanban:** the board's agents carry `name`, `uid`, `block_id` and `state`, but no colour.
- **Column colour:** only Needs you has one (`--warning-color` border). Cards in Needs you get a `--warning-color` left border.
- **Narrow layout:**
  - **Breakpoint:** `@container warden (max-width: 760px)` (`kanban.scss`) turns the grid into one column, with Needs you first.
  - **Scrolling:** each column list keeps `overflow-y: auto` inside a height-limited board, so the stacked columns squeeze and scroll one by one. Read from the CSS, not seen.
  - **Lanes:** in Lanes mode the overrides already make one scrollbar (`.kanban-lanes`).
- **Drag and drop** (`@atlaskit/pragmatic-drag-and-drop`):
  - Backlog cards can be dragged onto a Backlog agent group (moves the task to that agent) or onto an agent chip in the strip (starts it there).
  - It works the same at every width.

## 4. Stages

### 4.1 What each column holds

Two kinds sets, defined once in `kanban-columns.ts` and in srv's `work_deps.rs`, matched case-insensitively:
- **Research kinds:** `research`, `spec`. A task that produces a document for the operator to approve.
- **Every other kind, or none:** treated as build work.

| Column | Holds | How it's computed |
|---|---|---|
| **Backlog** | Waiting tasks that are not Spec'd: requests, research and spec tasks not yet started, builds whose spec isn't accepted yet, plain tasks | `open` or `held`, and not Spec'd |
| **In Research** | A research or spec task being worked | `claimed`, kind in research kinds, holder fine (as In progress is today) |
| **Spec'd** | A task whose spec or report was accepted and that nobody has started | `open` or `held`; it starts after at least one task of a research kind; every task it starts after is finished; and at least one of those research-kind tasks is `done` (accepted), not `cancelled` |
| **In progress** | Build work and every other task being worked; untracked agents | `claimed`, kind not a research kind, holder fine |
| **Needs you** | As today: a question, a failure, a holder waiting, errored or gone | unchanged; it overrides the stage |
| **Review** | A task whose PR is in review (muxreview), or one handed in for a person; specs and builds, the card says which (§4.4, §4.5) | `review`; or `claimed` with a PR link whose merge srv hasn't seen, holder fine |
| **Merged** | Finished work: its PR merged, or it was completed with no PR; cancelled tasks in a folded "Cancelled" group at its foot | `done` not yet released; `claimed` whose PR merged; `cancelled` |
| **Released** | Merged work that has shipped in a release | `done` whose PR's merge is in a release (§4.5) |

**Examples:**
- **#1103 "Spec: Kanban stages"** (kind `spec`): Backlog → In Research when I claim it → Review when I submit it → Merged when the operator accepts it.
- **#1106 "Build: Kanban stages"** (kind `build`, after #1103):
  - Backlog with "after #1103", while #1103 is anywhere before Merged;
  - **Spec'd** once #1103 is accepted;
  - In progress when claimed, then Review while its PR is in review, Merged when it merges, Released when a release ships it.
- **A task with no kind and nothing it waits on** goes Backlog → In progress → Review → Merged → Released. Small fixes never pass through In Research or Spec'd (§5.3).
- **A task with no kind that starts after an accepted spec** is Spec'd. The build's kind doesn't matter, only its predecessor's.
- **A Spec'd task the operator puts on hold** stays in Spec'd, greyed with "on hold", like a held card in Backlog. Spec'd means "approved, not started"; the hold is a separate signal.
- **A research task with no follower** goes Review → Merged. Only a task that waits on it can be Spec'd.

### 4.2 Why Needs you stays

The operator's lists don't name it, but it is the only column that asks the operator for something: a question, a failure, a closed agent. Its count drives the Warden tab title and the widget-bar badge (`needsCountOf`). Folding it into In Research or In progress would hide the one thing the board exists to surface. It keeps its place after In progress and before Review, and a card in it shows its stage as a word ("spec", "build"). D2 asks the operator to confirm.

### 4.3 Derived, not stored

Every column is computed by `columnOf` from what the board already has, plus one field, for these reasons:
- **No new state:** the claim query, the reaper and older builds sharing the store (`SPEC_WARDEN_KANBAN_2026_10_20.md` §10) are untouched. An older build shows a Spec'd task in Backlog, and an In Research task in In progress.
- **Nothing to keep in sync:** a stored "spec'd" state would have to be set on accept, cleared on a send-back or retry, and repaired after an older build claims the task. A computed column is always right.
- **The one srv change:** `AfterTask` gains `kind` (one more column in `after_of`'s join), so `columnOf` can see what a task waits on.

What `columnOf` gives up is a way to mark a task Spec'd without a spec task before it. That's deliberate: Spec'd means the operator accepted a document.

### 4.5 Review is muxreview; Merged; Released (the operator, later on 2026-10-23)

> "Review in Kanban is essentially muxreview ... I review via casual dogfooding, not in the kanban."
> "lets change 'Done' to 'Merged'. then add an addl final column Released."

And the working rule that followed (Global Memory "Agent seats"): agents file tasks with `review: false`, keep a task claimed with its PR link while the PR is in review, and `complete` it when the PR merges.

- **Review** shows what is in review on GitHub: a held task with a PR link sits there from the moment its PR is linked until srv sees it merged. A task handed in for a person (`review` state) still sits there too, and the operator's Accept still works, but it is no longer the normal way out: a merge is.
- **Merged** is finished work. A task the agent completed is there, with or without a PR; so is a held task whose PR srv saw merge before its agent completed it, so the board doesn't wait on the agent. Cancelled tasks sit under it in a folded "Cancelled · N" group, out of the way, never in Released.
- **Released** is merged work that has shipped. How srv learns it:
  - **The merge.** The PR watcher (`backend/kanban/prs.rs`) asks GitHub about every task with a PR whose merge it hasn't seen: in Review, held, or completed. When the PR has merged it records when (`db_work_prs.merged_at`); a task in `review` is accepted as before.
  - **The release.** For each repository with merged, unreleased tasks, at most every 30 minutes, it reads the repository's releases. A release's `created_at` is the date of the commit it was made from, and `main` is linear (a merge queue), so the first release (not a draft) made at or after a PR's merge contains it. The task is recorded as released in that tag (`db_work_prs.released_in`), with the release's publish time.
  - **The limit.** srv holds no GitHub credential: this reads GitHub's public API, within the PR watcher's existing hourly budget. A private repository's tasks reach Merged when their agent completes them and never reach Released; the board says nothing it can't know.
- **The board's window:** finished tasks stay on the board for 7 days after they last changed, as before; a release records a change, so a task stays a week after it ships.

### 4.4 Showing how the two tasks are tied

- **On the build card:**
  - A "spec #1103" pill (icon `file-lines`) replaces the hourglass "after #1103" pill when the task it waits on is a research kind.
  - The pill shows the spec's state: "spec #1103 · in review", or "spec #1103 · accepted".
  - Clicking it opens the spec's drawer.
- **In the build's drawer:** "Starts after" lists the spec as today. The spec's document path (§5.2) shows under it, with an **Open** button that opens the file in the Editor beside the Kanban (`Pane` `open`'s path).
- **On the spec card:** a "build #1106" pill, so the operator sees in Review what accepting it unlocks. srv adds `followers: AfterTask[]` to the card, read through `db_work_deps`'s existing `after_id` index.
- **In Review:**
  - A spec card's **Accept** button reads **Accept spec**, and its tooltip reads "Unlocks #1106 'Build: …' for Naki-2".
  - The Review header splits its count by stage: "4 · 1 spec · oldest 2 h".
  - Review's soft limit (`kanban:reviewlimit`) still counts both.

## 5. Filing a feature

### 5.1 One call for the pair

`Tasks` `enqueue` gains `first: "spec" | "research"`. With it, srv files two tasks in one transaction and answers with both numbers:
- **"Spec: <title>":**
  - kind `spec` (or `research`);
  - payload: the given payload, plus a line saying to write a spec (or report) and leave it uncommitted, then `submit` with its path in `links.doc`;
  - review required.
- **"Build: <title>":**
  - kind `build`;
  - payload: the given payload, plus a line saying to build what the accepted spec says;
  - starts after the spec;
  - same target, repo and priority;
  - review as `review` says.
- **The answer:** `Filed #1103 "Spec: …" and #1106 "Build: …" (starts after #1103) for Naki-2.`

Without `first`, `enqueue` files one task, as today.

**The operator's New task** gets a "Spec first" check box, ticked by default. The operator described specs-first as how they work, so it is the default and they untick it for a small fix (D4). It files the same pair through `kanban.act` `create`.

**The operator-config guidance** (`crates/srv/operator-config-seed.json`, the `Tasks` paragraph) gets one sentence: "A new feature is two tasks: file it with `first: "spec"`; a fix, a small change or a follow-up is one task." Global Memory's "Agent seats" entry already says the same in prose, and this makes it the tool's own guidance.

### 5.2 The spec's path

- **Where it goes:** `submit` and `update` take `links.doc`, a file path, normally the uncommitted spec in the agent's clone.
- **Storage:** a new table beside the board's, created with the others and with no schema version bump: `db_work_docs (work_id PRIMARY KEY, path, updated_at)`.
- **Display:** the card and drawer show the path (§4.4). The path is display text and an Open button, never fetched by srv.
- **The build's agent** reads the spec from that path. The workspace rule says a spec ships in the same PR as its code, so the build commits it.
  - When the build is assigned to the agent that wrote the spec (the default), the file is already in its clone.
  - When the operator moves the build to another agent, that agent reads the file where it is and copies it into its own clone. Reading another workspace's file is allowed; changing it is not.

### 5.3 A feature with no spec

A task filed without `first` or "Spec first", and without an `after` on a research kind, never enters In Research or Spec'd. That covers bug fixes, one-line changes and follow-ups, as today. Whether something deserves a spec stays the filer's judgement and the operator's, not srv's.

## 6. Approval rules srv enforces

1. **A spec doesn't skip Review.**
   - **The rule:** on `submit` or `complete`, a task of a research kind with at least one follower that isn't finished goes to Review, whatever `review_required` says. The answer says so: "Review: #1106 starts after it, so the operator accepts it first."
   - **Why there:** it is enforced at hand-in, the one moment it matters. It still holds when the follower was added after the spec was filed, which an enqueue-time default would miss.
   - **The operator** can still Accept, or remove the dependency in the build's drawer.
2. **Accepting a spec tells the build's agent.**
   - **When:** when Accept (or a merged PR, §6.9 of the Kanban spec) leaves a follower with nothing unfinished to wait on.
   - **The message:** srv sends that follower's agent one message: `Task #1106 "Build: …" can start: the operator accepted its spec #1103 (<path>). <operator's accept comment, if any>. Take it with Tasks claim (id 1106) when you've finished your current task.`
   - **When it isn't sent:**
     - when the agent's block has `kanban:continue: false`;
     - when the follower is held (the hold keeps it unclaimed);
     - when the follower is for Anyone or a group.
   - **An agent that isn't running** gets it through the message hold, like any other message, for up to 24 h. After that, the card waits in Spec'd under its name.
   - **Why:** today a newly free task waits until its agent happens to finish something else.
3. **A send-back keeps the build waiting.** The spec returns to In Research (its agent) or Backlog (agent gone), as today. The build stays in Backlog with "spec #1103 · sent back". Nothing new is needed; this is how "starts after" already behaves.
4. **Cancelling a spec doesn't release its build.** Cancel on a research-kind task with unfinished followers asks once: "Also cancel #1106 'Build: …'?", ticked by default.
   - **Ticked:** both are cancelled, each with a history row.
   - **Unticked:** the build is put on hold, with the reason "its spec #1103 was cancelled". It sits in Backlog, and the operator decides what happens to it: unhold it to build without a spec, edit what it waits on, or cancel it later.
   - **Why:** today the build would become claimable the moment its spec is cancelled, which is the opposite of what cancelling a spec means. D5.
5. **A failed spec** holds its build, as today.

The board moves stay operator-only, as `SPEC_WARDEN_KANBAN_2026_10_20.md` §6.13 says: Accept, Send back, Cancel, and moving or starting tasks. Agents gain only `first` and `links.doc`.

## 7. Colours

### 7.1 Each column's colour

There are eight new `:root` tokens in `frontend/app/theme.scss`, aliases of tokens each theme already tunes. Most are the `--process-kind-*` colours, which the Tower's test keeps apart and readable in every theme. The accent colour isn't used: it differs in every theme and met another stage in most of them.

| Column | Token | Alias | Why |
|---|---|---|---|
| Backlog | `--kanban-stage-backlog` | `--secondary-text-color` | Neutral: nothing is happening yet |
| In Research | `--kanban-stage-research` | `--process-kind-agentmux` (cyan) | |
| Spec'd | `--kanban-stage-specd` | `--info-color` (violet) | |
| In progress | `--kanban-stage-progress` | `--process-kind-agents` (magenta) | |
| Needs you | `--kanban-stage-needs` | `--attention-color` | The app's own "an agent needs you" colour. Today the Kanban uses `--warning-color` here; this aligns it with the agent pane's attention signal |
| Review | `--kanban-stage-review` | `--process-kind-builds` (yellow) | |
| Merged | `--kanban-stage-merged` | `--process-kind-shells` (green) | Finished |
| Released | `--kanban-stage-released` | `--process-kind-runtimes` (blue) | Shipped: the last column, set apart from Merged |

Seven themes override one or two of these where two still met, or one didn't read on its surface (catppuccin, high-contrast, light, midnight, monokai, solarized-light, tokyo-night), each in a small block at the end of its theme file.

- **Where the colour shows:**
  - a 3 px top bar on the column's header;
  - a dot before the column's label;
  - in the narrow view, the section header's left bar.
- **What stays uncoloured:** column bodies and cards. The Kanban's rule is borders, not fills (`kanban.scss`'s header note), and fills would fight the agent colours on the cards.
- **Themes:** a theme where two of these collide or don't read on its surface overrides the alias in its own `themes/*.scss`. A test (`view/warden-kanban/kanban-stage-colours.test.ts`), modelled on `view/tower/process-kind-colours.test.ts`, checks two things in every theme, light and dark (a light theme with theme.scss's light-polarity block applied):
  - the eight stay apart (a minimum OKLCH distance);
  - each has at least 3:1 contrast against `--main-bg-color` (WCAG 1.4.11 for indicators).

### 7.2 Each card's agent colour

- **The agent's colour, as its pane shows it:**
  - **Card:** a 3 px left stripe in the agent's `border` role colour.
  - **Agent chip:** its dot in the `identity` role colour.
  - **Backlog and Spec'd agent groups:** the group header gets a dot in the agent's colour too.
- **How the frontend gets it:**
  1. **The agent has a pane here:** `blockRoleColor(blockMeta, isLightThemeActive(), role, { widget: false })` on its block (`KanbanAgent.block_id`), exactly as Swarm and Tower do. This includes a colour the operator picked in the pane's Pane Color menu.
  2. **No pane here** (closed, another instance, not registered): srv sends the stored `ui:color` hex on the board's agent rows. The frontend passes it through the same role mapping, as `{ "frame:activebordercolor": hex }`, so it is theme-tuned the same way.
     - **Which agents:** srv adds `color` to `KanbanAgent`, and an `agents_by_uid` map on the board for each task's `target_agent_uid` and `claimed_by_uid` that has no registration. The lookup is the uid's `ui:color` entry; the uid is the definition id.
  3. **Neither:** a task targeted by name only, a group, or Anyone. There's no stripe colour: the card uses `--border-color`.
- **The fallback:** not Tower's `pickAgentColor(name)`. It hashes the name, while the stored colour was hashed from the definition id, so it would show a colour the agent doesn't have.
- **Untracked cards** carry their agent's colour the same way, since they have a block.

### 7.3 How colour combines with the status signals

The rule, already the Kanban's and the UI kit's (`Pill.tsx`, `SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md`): colour reinforces, and words and icons carry the meaning (WCAG 1.4.1). Each card:
- **Agent:** stripe plus chip, and the agent's name is always written in the chip.
- **Stage:** the column it's in. In the narrow view, the section header names it. On cards in Needs you and Review, a word pill: "spec", "research" or "build".
- **Needs you:**
  - The `--warning-color` left border is removed: the left edge is now the agent's.
  - The card gets a full 1 px border in `--kanban-stage-needs`.
  - Its reason line ("asks: …", "failed after 3 attempts") gets a `circle-exclamation` icon and stays the first line.
- **On hold:** unchanged: the card is at 60 % opacity (its stripe included) with a "pause" pill reading "on hold".
- **Starts after:** the hourglass pill "after #N", or the "spec #N · state" pill (§4.4). Unchanged in meaning.
- **Review over the soft limit:**
  - Today the count turns amber, which would vanish on Review's own yellow header.
  - It becomes words plus an icon: "5 · over 4" with `triangle-exclamation`, in `--warning-color` and bold.
  - The text alone says it.

### 7.4 The CI checks this must pass

- **`scripts/check-ui-primitives.mjs` (the UI primitives ratchet; rule 10, colour literals):** the eight tokens are aliases set on `:root`, which the rule exempts. Component SCSS uses only `var(--kanban-stage-*)` and existing tokens, with no `#hex`, `rgb()` or `hsl()`. Per-file counts must not rise.
- **Fallbacks:** don't give a `var()` fallback to a token that is always set on `:root`; the same script counts those. The agent colour is set inline as a CSS variable from TSX (`--kanban-agent-color`, as Swarm sets `--swarm-agent-*`), and its fallback, `var(--kanban-agent-color, var(--border-color))`, is legitimate because that variable is not on `:root`.
- **`npm run lint:theme-colors`:** no Tailwind palette utilities in the TSX.

## 8. The narrow view

- **Breakpoint:** unchanged: the Warden container at 760 px or less (`@container warden (max-width: 760px)`). It becomes a named SCSS variable, `$kanban-narrow`, and `KANBAN_NARROW_PX` in `kanban-columns.ts`, which the manager's resize observer compares the section's width with to fold Merged and Released.
- **Wider than that:**
  - Eight columns at `minmax(160px, 1fr)` need about 1,340 px.
  - Between 760 px and that width, the board scrolls sideways, as five columns do today below about 940 px.
  - All eight columns are always shown, empty or not: fixed positions are what make a board readable at a glance.
- **One list:**
  - **The scroller:** the board becomes the single vertical scroller (`overflow-y: auto`).
  - **The columns:** each becomes a section with `overflow: visible` and no height limit. These are the overrides Lanes mode already uses.
  - **The headers:** each section opens with its header: the stage bar on the left, the label, the count.
  - **Sticky:** the header is `position: sticky; top: 0`, so the section being read is always named.
  - **Empty sections:** collapse to their header line ("Spec'd · 0").
- **Order:** the column order, with Needs you first, as today: Needs you, Backlog, In Research, Spec'd, In progress, Review, Merged, Released. Needs you is the one section the operator must not have to scroll to (D3).
- **Merged and Released** are folded by default in the narrow view (one header line each, with its count). Tapping a header opens it, for as long as the pane is open.
- **Lanes mode, narrow:**
  - One scroller for the whole list.
  - Each repository lane is a heading (sticky at top), and its stage sections follow it in the same order.
  - Section headers inside a lane aren't sticky (two sticky levels overlap on a short pane); the lane header names the repository and its count.
- **Drag and drop:**
  - It stays on. The targets are the same: Backlog and Spec'd agent groups, and the agent strip.
  - The strip stays above the scroller, so it's always reachable.
  - Dragging to a group off screen needs the list to scroll: Chromium scrolls a scroll container during a native drag near its edge, which pragmatic-drag-and-drop uses. The build checks it in a narrow pane.
  - If it doesn't scroll, add `@atlaskit/pragmatic-drag-and-drop-auto-scroll` (same family) on the board scroller.
  - The card menu's "Move to…" and "Start on…" remain the way without a mouse.

## 9. Elsewhere in the app

- **Widget-bar badge and Warden tab title:** still the Needs-you count only. Spec'd and In Research ask nothing of the operator.
- **Swarm:** a row's task chip ("#12 …") gains the stage word when it's research ("#1103 · spec") and a dot in the stage colour. Swarm rows already carry the agent's colour.
- **The route-name target bug (#1257).** A pair's build must reach its owner, and today it may not:
  - **The bug:** `target_agent` given as a display name's route form ("Naki-2") resolves to no agent. It is stored as a bare name that nobody's claim matches. This is how #1103 and #1106 got stuck.
  - **The fix (#1257, separate):** the resolver matches the route form, and `enqueue` refuses a target that matches no agent.
  - **For the pair (§5.1):** both tasks are resolved together and refused together, so it never files half a pair, or a pair nobody can take.

## 10. Files

**Frontend** (`frontend/app/view/warden-kanban/`):
- **`kanban-columns.ts`:** `COLUMNS` (eight), `columnOf` (§4.1, §4.5), `RESEARCH_KINDS`, `isSpecd`, stage words, `reviewLoad` split by stage.
- **`kanban-board.tsx`:** the Spec'd column groups by agent like Backlog, and its groups are drop targets; column header dot and bar; narrow sections with sticky headers and the Merged and Released folds.
- **`kanban-card.tsx`:** agent stripe and chip colour (§7.2); "spec #N" and "build #N" pills; Needs-you border and icon; stage pill.
- **`kanban-drawer.tsx`, `kanban-after.tsx`:** the document path and Open; the "Accept spec" label and tooltip; the cancel confirmation with "Also cancel #N".
- **`kanban-new-task.tsx`:** "Spec first".
- **`kanban-types.ts`:** `KanbanAfter.kind`, `card.followers`, `card.doc`, `KanbanAgent.color`.
- **`kanban.scss`:** the stage tokens in use, the narrow list, removing the `--warning-color` Needs-you left border.
- **Elsewhere:**
  - `frontend/app/theme.scss`: the eight aliases.
  - `frontend/app/themes/*.scss`: overrides where the test asks for them.
  - Swarm's task chip.

**srv:**
- **`backend/storage/work_deps.rs`:** `kind` on `AfterTask`, `followers_of`, `RESEARCH_KINDS`.
- **`backend/storage/work_board.rs`:** `db_work_docs`, the `doc` field on the card, and `followers`.
- **`server/work_tasks.rs`:** the hand-in rule (§6.1); after a finish, the follower message (§6.2), shared with Accept.
- **`server/work_queue.rs`, `crates/mcp/src/tool_schemas.rs` and the Tasks tool:** `first` and `links.doc`.
- **`server/app_api/kanban.rs`:**
  - `create` with the pair;
  - `accept` sends the follower message;
  - `cancel` with `also_cancel_followers`, or holds them;
  - agents' `color` and `agents_by_uid`.
- **`crates/srv/operator-config-seed.json`:** one sentence in the Tasks guidance. The manifest version bumps.

**Docs:**
- this spec, with `SPEC_WARDEN_KANBAN_2026_10_20.md`'s sections listed under Changes;
- agentmux-docs' Kanban page, in a later pin bump.

## 11. Tests

- **`columnOf`, one case per row of §4.1:**
  - each kind (`spec`, `Spec`, `research`, `build`, none);
  - a build after a spec in each of the spec's states (open, claimed, blocked, review, sent back, done, cancelled, failed);
  - a build after a spec and a plain task, with one finished and one not;
  - held and Spec'd;
  - Needs you overriding In Research;
  - an older srv's board with no `kind` on `after` (falls back to Backlog).
- **srv:**
  - a research-kind task with an unfinished follower goes to Review on `complete` even with review off, and goes straight to done with no follower;
  - Accept sends exactly one follower message, and none for `kanban:continue: false`, a held follower, Anyone, or a follower still waiting on another task;
  - a cancelled spec, both ways (followers cancelled, followers held with the reason);
  - `enqueue` with `first` files both tasks in one transaction, and a refused target files neither;
  - `links.doc` stored and returned.
- **Colours:**
  - **The new theme test:** every theme, light and dark, keeps the eight stage colours distinct and at 3:1 against the surface.
  - **The resolver:** block meta first, then `color`, then none, never the name hash.
  - **The ratchet** stays at or below the base branch.
- **Narrow view (component tests on the board at 700 px):**
  - one scroll container;
  - no list inside it has `overflow-y: auto`;
  - section order;
  - sticky headers;
  - Merged and Released folded;
  - drop targets still registered.
- **By hand, in an isolated `task dev` build:** two themes (one light, one dark), the board wide and narrow, a pair filed, the spec accepted, the build moving to Spec'd, and the message reaching the agent.

## 12. Rollout

All of it is build #1708 (which replaces #1106), in three PRs, each shippable on its own:
1. **Stages:** the eight columns (§4, §4.5): `columnOf`, the card's links between a spec and its build, and srv's `db_work_prs` with the PR watcher's merge and release checks. This spec ships with it.
2. **Colours and the narrow view:** §7, with the theme test, and §8.
3. **Filing and approval rules:** §5 and §6: `first`, `links.doc`, the hand-in rule, the follower message, cancelling a spec.

**Compatibility:**
- No migration. The new tables (`db_work_prs`, `db_work_docs`) are created on open.
- An older build on the same computer ignores `db_work_docs`. It shows Spec'd tasks in Backlog and In Research tasks in In progress, and its claim doesn't know the hand-in rule, the same as with today's board states.
- The Kanban page in agentmux-docs is updated after the release, with the next pin bump.

## 13. Decisions for the operator

| | Decision | Recommended |
|---|---|---|
| D1 | Columns derived from kind and "starts after", not a stored state | **Derived** (§4.3) |
| D2 | Needs you stays as its own column, between In progress and Review | **Yes** (§4.2) |
| D3 | Narrow view order: Needs you first, then the column order | **Needs you first** (it's the one section that asks you something) |
| D4 | The New task box "Spec first" is ticked by default | **Yes**, untick for small fixes |
| D5 | Cancelling a spec: also cancel its build (ticked by default), else hold it | **Yes** |
| D6 | Accepting a spec messages the build's agent at once | **Yes**, except with `kanban:continue: false` |
| D7 | Needs you's colour becomes the app's magenta attention colour instead of amber | **Yes**, it matches the agent pane's own "needs you" |

## 14. Sources

- **The operator's requests:** quoted in §1, from AgentY's messages to Naki-2 on 2026-10-23.
- **Specs:**
  - `docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md`
  - `SPEC_KANBAN_AGENTS_HOLD_AND_UNHOLD_2026_10_22.md`
  - `SPEC_KANBAN_REMINDER_OFF_BY_DEFAULT_2026_10_22.md`
  - `SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md` (colour paired with the name)
  - `SPEC_TOWER_THEME_SURFACE_AND_COLOR_2026_10_17.md`
- **Code:**
  - Kanban frontend: `frontend/app/view/warden-kanban/*`
  - Kanban srv: `crates/srv/src/server/app_api/kanban.rs`, `server/work_tasks.rs`, `backend/storage/work_board.rs`, `work_deps.rs`
  - Agent colours: `backend/agent_color.rs`, `frontend/app/block/pane-identity.ts`, `pane-color-scheme.ts`, `view/swarm/swarm-row-colors.ts`, `view/tower/tower-agents.tsx`
  - Theme and checks: `frontend/app/theme.scss`, `scripts/check-ui-primitives.mjs`, `eslint.theme-colors.config.js`
