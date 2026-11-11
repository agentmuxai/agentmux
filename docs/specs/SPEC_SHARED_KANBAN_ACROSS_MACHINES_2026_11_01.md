# SPEC: Shared Kanban: one board across a user's machines, by LAN or AgentMux Cloud

**Status:** active, being built (B1a in PR #6872). **Approved** (operator, 2026-11-08, relayed by AgentY; verbatim: "ok, shared kanban is good, but does it consider having multiple kanbans? we need to be able to have multiple kanbans, and select them in the kanban view", then "ok, greenlight kanban work"), revised 2026-11-08 for the one change the operator asked for: **several boards in the first build, with a board picker in the Kanban view** (§9). Spec task #17016, Naki-2. The recommendations in §14 stand unless the operator says otherwise.
**Author:** Naki-2@narko.
**Trigger (operator, 2026-11-01):** "I just have 2 ideas: 1) Managed vault 2) Managed Kanban .. in both cases, users of AgentMux Cloud can use kanbans and vaults from their own cloud", then "users should still have this ability to shared via the LAN tier, without cloud.", then "the other question of whether users can have multiple vaults/kanbans", then "their own cloud means an agentmux hosted service", and on names "Kanban and Vault will simply retain their names".
**Verified on:** `main` at `4dfaa78abd` (2026-11-08), AgentMux 0.60.21. Re-checked 2026-11-08 by Naki-2@narko against the draft written at `e4f374c61` (2026-11-01): the Kanban storage and remote-ops files this spec relies on (`work_queue.rs`, `work_board.rs`, `work_deps.rs`, `work_claims.rs`, `work_remote.rs`) have no changes since, the card table still numbers per machine (`num INTEGER PRIMARY KEY AUTOINCREMENT`), the card and its types still carry no host or machine, and no `collection_id` exists yet. Since the draft: the Vault spec was approved (operator, 2026-11-04) and its first steps (several Vaults on one computer, VS1a and VS1b) are built, so the Vault's sharing step waits on this layer; the remote-ops module grew (`remote_ops/op.rs`) with more ops, which does not change the design.
**Depends on:** `SPEC_USER_DATA_SYNC_LAYER_2026_11_01.md` (the sync layer both the Kanban and the Vault use; "the layer" below). **Extends:** `SPEC_WARDEN_KANBAN_2026_10_20.md` (§6.11 says the board is per machine and "other computers … are Phase 4"; this is that phase) and `SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` (§4.3, `tasks.enqueue` and `tasks.list` with `host`).

## 1. Summary

Today the Kanban is per machine. A task for an agent on another machine goes by message, or by `Tasks enqueue` with `host` onto that machine's own board, and the same `#number` means different tasks on different machines. This spec makes **one board a user's machines share**, in the three tiers of the layer:
1. **Local**: as today, one machine.
2. **LAN**: a user's machines share the board peer to peer, with no cloud.
3. **AgentMux Cloud**: the board also syncs through the hosted service, for machines that are never on one network.

**A person has several boards** ("Main", "Work", "Client X"…), each its own collection with its own members, tier, private or readable switch and agent list, picked in the Kanban view; today's board becomes the default board "Main" and keeps every number (§9).

The design in one paragraph: **content is shared by everyone, lifecycle belongs to one machine.** Any member edits a task's title, notes, links and order, and the layer merges those field by field. A task's *lifecycle* (who claimed it, its state, its attempts and lease) is decided only by the task's **home machine**, the one where its target agent lives, so a claim never needs two machines to agree and today's lease, attempt fence and reaper carry over unchanged. Numbers stay unique because each machine leases blocks of them from the board's numbering authority. Today's per-machine boards merge without renumbering what people refer to.

## 2. What exists (verified in code)

- **Schema** (identity store, per machine): `db_work_queue` (`id` UUID, `title`, `payload`, `kind`, `target_agent`, `target_group`, `state` open | held | claimed | blocked | review | done | failed | cancelled, `claimed_by`, `claimed_by_uid`, `claim_expires`, `attempts`, `max_attempts`, `created_by`, `created_by_uid`, `not_before`, `result`); `db_work_cards` (`num` INTEGER PRIMARY KEY AUTOINCREMENT, `work_id` UNIQUE, `repo`, `branch`, `pr_url`, `rank` REAL, `review_required`, `checklist`, `question`, `note`, `requested_by`, `requested_by_kind`, `request_text`, `filed_by`); `db_work_events` (append-only history, `actor`, `actor_kind`, `kind`, `from_state`, `to_state`, `body`); `db_work_deps` (`work_id`, `after_id`; at most 10 per task).
- **Numbers:** `num` is per machine; the `Tasks` tool takes a UUID or `#num` (`id_or_num`). People and messages say `#168`. There is no `host` column.
- **Claims:** an atomic claim sets `state='claimed'`, `claimed_by`, `claimed_by_uid`, `claim_expires = now + lease` (default 2 minutes) and `attempts += 1`; heartbeat extends the lease; a reaper reopens an expired claim (or fails it at `max_attempts`); every heartbeat, complete and release carries the **attempt** from the claim (an ABA fence) so a stale call can't touch a newer claim. The holder is matched by agent uid.
- **Across machines today:** `tasks.enqueue` and `tasks.list` as remote ops (`work_remote.rs`): the target is named by slug and resolved on the owning machine (`resolve_typed_name`; ambiguous or missing is refused with candidates); the task is recorded as filed by `slug@machine` (kind `agent`), never the operator; no `target_group`.
- **The pane** shows agent name, provider icon and state; **no host or machine appears** in the task schema or the card (§6.11 of the Kanban spec says other computers are Phase 4).
- **The queue is shared by every AgentMux on one computer** (the identity store, `work_queue.rs`): a per-channel board was rejected on purpose. The shared board keeps that.

## 3. Model

### 3.1 A board is one collection, and a person has several

Each board is a layer collection of kind `kanban` (id, name, slug, tier, members, key, agent list, readable switch). A new table `db_boards` on each machine lists the boards that machine is a member of; every task table gains a `board_id`. A machine sees only the boards it is a member of: a board shared by two of a person's machines does not exist on a third. One board is the **default board** (§9.3). Everything below (numbers, homes, claims, merge, media) is **per board**: where this spec says "the board", it means the board the task is on.

### 3.2 Task numbers that stay unique across machines

**Numbers are per board** (decided in §9.1). Today's `num` comes from one machine's autoincrement; on a shared board:
- **The numbering authority** of a board is one of its members (the machine that created the board; for "Main" that is narko). It owns that board's counter.
- **Blocks:** every member machine leases a block of 50 numbers from the board's authority when it joins and again when its block runs below 10. A task gets its number from the machine's block when it is filed, and keeps it forever. Numbers are unique within the board, increase on each machine, and are not in time order across machines (a gap or an out-of-order `#` is normal).
- **Offline with no block left:** the task is filed with a provisional ref, `#pending-<machine>-<n>`, usable in every `Tasks` call on that machine, and receives its real number the moment the machine reaches the authority. The provisional ref keeps working as an alias.
- **If the authority is unreachable for good,** any member may take over the counter after the person approves in the host window (the next number is above every one it has seen).
- **Reading a number:** `#168` means task 168 **on the default board**, exactly as today. On another board a task is `work#12` (the board's slug, `#`, the number). Every answer, card and message prints the full form for a task not on the default board.

### 3.3 Home machine and who may change what

Each task has a **home machine**, fixed at filing (and changeable only by the hand-off of §4.4):
- A task **with a named target agent** is homed on that agent's machine. The task goes to the agent without a message, and its claim is a local call there.
- A task with **no target** is homed on the machine that filed it, and by default claimable only by that machine's agents (today's behaviour). A task can say `scope: "anyone"` to let any member machine's agents claim it (the claim is then a remote call to the home machine, §4.2).

| Part of the task | Who may change it | Merge |
|---|---|---|
| **Content:** title, payload (until first claimed), repo, branch, PR, checklist, question, note, priority, `not_before`, links, review requirement | any member | per field, newest wins; the overwritten value stays in the history |
| **Order:** `rank` (the operator's backlog order) | any member | newest wins (a fractional rank with a machine tiebreak) |
| **Lifecycle:** state, claimant, lease, attempts, result, accept and send-back | **the home machine only** | the home machine's decision is carried; no merge |
| **Dependencies:** `after` | any member adds; removing is a tombstone | add-wins set |
| **History:** `db_work_events` | any machine appends its own | append-only, keyed `(origin, sequence)`; no conflicts |
| **Media** | any member adds | content-addressed, add-only (§5) |

## 4. Claims, leases, targets

### 4.1 An agent on the home machine (the common case)

Unchanged. Claim, heartbeat, complete and release are today's local atomic updates. The layer replicates the resulting state to the other members as the home machine's decision.

### 4.2 An agent on another machine claims a task (`scope: "anyone"`, or a reassigned one)

The claim, each heartbeat, the completion and the release go to the home machine as remote ops (`board.claim`, `board.heartbeat`, `board.complete`, `board.release`), over the same proof as every remote op. The home machine runs today's atomic claim with the remote agent as claimant `(machine, uid)` and returns the attempt, which the remote agent keeps as its fence. The lease and the reaper are the home machine's.
- **The claimant's machine goes offline or the agent dies:** the lease lapses on the home machine, the reaper reopens the task (or fails it at `max_attempts`), exactly as today.
- **The claimant is only partitioned and finishes:** its late `complete` carries a stale attempt and is refused; the agent is told the claim was lost. Nothing is merged wrongly.
- **The home machine is offline:** the remote agent can't claim, and the answer says "this task's home is Area54, offline since 10:12". A running claimant keeps working; its completion is queued as an op for the home machine, with its fence, and applied when the home returns (refused if the lease lapsed and the task was reclaimed meanwhile).

### 4.3 `target_agent` across hosts

- A target is `slug` or `slug@machine`. Each machine publishes a small **roster** into the layer (slug, uid, name, seat, whether the agent is running); the board resolves a target against the members' rosters, as `resolve_typed_name` does on one machine today. A slug on more than one member machine is refused with the candidates ("lazo@Area54 or lazo@charlie").
- `Tasks enqueue` with `host` stays as sugar for "target that machine's agent" for as long as the shared board is off or the machine isn't a member; on a shared board it files onto the shared board.
- A target that doesn't exist yet on any member is refused at filing, as on one machine today. **A target must be on the board's agent list** (§9.4): filing on a board for an agent that can't see it is refused with the reason ("lazo isn't on board Work's agent list").

### 4.4 Home hand-off

If a home machine stays offline beyond a limit the person sets (default 12 hours), a member may **take over the home** of that machine's *unclaimed* tasks, after the person approves in the host window; the home returns on its next contact and the board shows the tasks' history. A *claimed* task is never taken over: its lease lapses and the reaper reopens it, which is the existing rule.

## 5. Media on cards

Today a card's media are files in the filestore, served by the machine. On the shared board a card's media are layer **blobs**: content-addressed chunks (the layer §3.4), referenced from the card by hash, name, type and size. A machine fetches a card's media on first view from any member that has them (LAN), or from the hosted service (cloud tier), and keeps them; a card shows a placeholder until then. The size cap per file is the collection's (default 25 MiB). A deleted card's media are removed on every member when the tombstone is acknowledged.

## 6. Dependencies, holds, review, the operator's columns

- **Spec and build pairs** (`first: "spec"`) are two tasks with an `after` edge, both homed by their targets. A build whose spec is homed on another machine starts when the home machine of the build sees the spec `done` in its replica; the replica lags the spec's home by seconds on a LAN, so the build may start slightly late, never early (a spec still replicating as `review` is not `done`).
- **Holds** (`hold`, `unhold`) are lifecycle: any agent or the operator asks, and the home machine decides, with no attempt consumed (as today).
- **Review** (`review_required`, accept, send-back): the requirement is content; accepting and sending back are lifecycle, done by the operator from any member's pane as a remote op to the home machine.
- **The operator's columns** (Backlog, Doing, Review, Done…) are derived from state and `after`, as today; the same board shows on every member, with each card's home on it (§7). **"Needs you" counts every board** the person can see: the picker (§9.2) shows each board's own count, and the badge in the header is the total, so a question on a board that isn't selected is never missed.
- **`after` stays inside one board** (the layer has no cross-collection references): a task can wait only on tasks of its own board.
- **Done and failed followers:** failed predecessors still hold followers (today's rule), on every machine.

## 7. The pane: which machine each agent is on

- Each agent chip shows `name · machine`, with the machine's state dot (online, offline since …, never synced).
- A card shows its home machine when it isn't the viewing machine, and "waiting for Area54 (offline since 10:12)" for a task whose home is offline.
- A **machine filter** next to the existing filters; a "this machine only" view for the operator who wants today's picture.
- The Kanban header carries the **board picker** (§9.2) at its left, before the filters.
- The board's header shows the tier and sync health (in sync, N changes waiting, last contact) with a link to the collection's console view.
- Swarm already shows remote agents with a platform tag; the board reuses that roster rather than a second source.

## 8. Migration: merging today's per-machine boards without renumbering what people refer to

**What people refer to is the narko board:** `#168`, `#16989`, `#17016` are the operator's and the leaders' numbers. Other machines' boards (Area54, starpower, charlie) are small and used by their stewards, and their numbers collide with narko's (every `#5` exists on several machines).

1. **Today's board becomes the default board "Main"** (slug `main`) on every machine, in the same step as `board_id` is added. **The first machine to share it becomes Main's numbering authority and keeps every number it has.** Its tasks join Main unchanged: UUIDs and numbers as they are, so every `#168` anyone ever wrote still means the same task. Boards a person made earlier on one machine (§9) stay on that machine until the person shares them.
2. **Every other machine's tasks are imported once**, when that machine joins:
   - **Open, held, claimed, blocked and review tasks** land on Main and get a new number from that machine's first block; the old local number is kept as `legacy_num` with the machine, and `#<old>@<machine>` (for example `#5@Area54`) resolves to the task for good. On the importing machine a bare `#5` typed by an agent that knows only the old number resolves through `legacy_num` for 90 days, with a note in the answer ("this is now #17042").
   - **Done, failed and cancelled tasks** are imported as history with their old number shown as `Area54 #5` (no new number is spent on them) and the same `#5@Area54` ref.
3. **Dependencies and links** between a machine's own tasks are rewritten to UUIDs (they already are in `db_work_deps`); a PR or task number written inside a task's text is left as written (it is the author's words).
4. **No task is lost or duplicated:** the import is keyed by the UUID (`id` is already unique), so re-running it, or two machines importing at once, is harmless.
5. **Mixed versions on one computer** (the Kanban spec §10): an older build on the same machine doesn't know the shared board and sees only the local tables; it keeps working on its own tasks and never sees remote ones. The migration doesn't remove anything an older build reads.
6. **`Tasks enqueue host:` and `tasks.list host:`** keep working; on a shared board `host:` is sugar for a target on that machine (§4.3).

## 9. Several boards per user (in the first build)

The operator (2026-11-08): "we need to be able to have multiple kanbans, and select them in the kanban view". Everything here ships in the first build, **locally first** (phase B1 of §13, before any sync, as the Vault's several local Vaults did), then travels over the layer as ordinary collections.

### 9.1 What a number means: per board, with `#168` kept for the default board

**Decision: numbers are per board; a bare `#n` is the default board's; another board's task is `slug#n`** (`work#12`).
- **Why not one global number:** a board may be members-only on one machine or shared with a different set of machines than another board. A single account-wide counter would force every board's members to talk to one authority, and a gap in the numbers would show how busy a private board is to machines that aren't on it. Per board, each board is self-contained: its own counter, authority and blocks (§3.2).
- **Today's `#168` keeps working with no change:** the default board "Main" keeps every number it has, and a bare `#n` is always the default board's. Messages that say `#168` still mean the same task.
- **The grammar:** a board has a **slug** (lowercase letters, digits and hyphen, up to 24, unique for the person, `main` for the default). A ref is `#n` or `slug#n`. `Tasks` accepts a UUID, `#n` or `slug#n` wherever it takes `id`. Renaming a board's display name never changes its slug, so refs keep working; changing the slug is a separate action that keeps the old slug as an alias for 90 days.
- **Cards and answers print the full ref** for a task not on the default board, with a copy button on the card.

### 9.2 Creating, renaming, archiving, deleting a board, and the picker

**The picker** is a button at the left of the Kanban header, in the kit's styles (the kit's menu and button, no new look): it shows the selected board's name, a small tier icon (this machine, LAN, cloud) and a count of what needs the person on it. Its menu lists the boards (each with its tier icon, its own "needs you" count and the default marked), then a divider, **"+ New board"** and **"Manage boards…"**. Archived boards sit in a collapsed "Archived" group at the bottom. The selected board is remembered per window. A board's view is today's Kanban view for that board, unchanged.

- **Create** (+ New board): name; sharing, one of **This machine only** (preselected: a board is private until the person shares it), **My machines (LAN)** or **My machines and AgentMux Cloud**; for a cloud board the private/readable switch; which agents see it (**All agents**, or a pick list). The creating machine becomes its numbering authority. A board shared on LAN adds the person's proven same-account machines as members (the join policy of §14.5).
- **Rename:** change the display name; the slug stays (§9.1).
- **Archive:** hides the board from the picker's main list and makes it read-only: no new tasks, no claims, running claims finish or lapse; it keeps syncing. **Unarchive** at any time. The default board can't be archived.
- **Delete:** only an **archived** board, and permanent: the board's records become tombstones that replicate, so every member machine removes it. A board with tasks shows the visible 15-second countdown with a veto, the operator's pattern for sensitive actions, not an approval gate; an agent may delete too, and the action is audited under the agent's name. The default board can't be deleted.
- **Set default:** any board can be made the default (see §9.3); the old default keeps its refs, and its bare `#n` become `main#n` in printed refs.
- **The same actions exist for agents** on the MuxAPI (§9.3), because nothing about them needs a human beyond the veto window.

### 9.3 The default board, and the MuxAPI's `board` parameter

- **The default board** is a per-person setting (initially Main). It is the board of every call that names none.
- **`Tasks` gains an optional `board`** (display name or slug) on `enqueue`, `list`, `claim`, `update` and the rest, **for native and container agents alike** (the route is the same srv route; a container agent reaches it through its host machine's membership, so it sees exactly the boards its host machine is a member of and the agent list allows). `id` also accepts `slug#n`.
  - `enqueue` without `board` files on the default board, and the answer says where ("filed as work#12" or "#201").
  - `list` without `board` lists the default board, and adds one line when more is waiting for the caller elsewhere ("3 more for you on Work and Client X"); `board: "all"` lists every board the caller sees, each task with its full ref.
  - `claim` without `id` considers **every board the agent sees**: its own targeted tasks first, then others by priority, the default board first on a tie; the answer names the board and the full ref. `claim` with an id or ref acts on that task only.
- **Board management on the MuxAPI:** `Tasks` actions `boards` (list, with each board's slug, tier, members, agent list, counts, default and archived flags), `board_create`, `board_edit` (name, sharing, readable, agent list), `board_archive`, `board_unarchive`, `board_delete`, `board_default`, and `move` (§9.5). One route each under `/agentmux/work/boards`, mirrored in the MCP tool and in the container route.

### 9.4 Each board's own members, tier, readable switch, and which agents see it

Each board has its own **members** (machines), **tier** (this machine, LAN, cloud), **readable by AgentMux Cloud** switch (cloud boards only; the default for a new cloud board is readable, §14.2) and **agent list**, all stored on the board's own collection and replicated by the layer, and editable from "Manage boards…" or the MuxAPI.
- **Which agents see a board:** the layer's §9 policy, unchanged: an agent sees a board if it runs on a member machine **and** the board's agent list (default: all) includes it. A call about a board the agent can't see is refused like an unknown id, and `list` never mentions it. A restricted board (an agent list of two) is how a team or a client project stays apart.
- **Members of a shared board** are chosen with the sharing option at creation and changed in Manage boards; adding a machine follows the layer's join policy (§14.5 here), removing one re-keys that board only.

### 9.5 Moving a task between boards

Moving is a **delete-and-create, with a marker and a link back**, as the layer's §9 says (no cross-collection identity):
- The task is created on the target board with a **new number from the target's counter** and the same title, payload, links and media (media by hash, not re-uploaded). Its first history line says "moved from main#168 by Name" with a link.
- On the source board the task becomes **moved**, a closed state shown in Done with "moved to work#12" and a link, and keeps its whole history.
- **A claimed task can't be moved** (release it first). **A task with `after` edges can't be moved either** while it has them (edges stay inside a board, §6); the answer lists them and offers "Move and drop the dependencies". Followers waiting on a moved task are told it was moved, and stay blocked until the person edits them.
- The target board's agent list applies: a task targeted at an agent that board doesn't list is moved without its target, and says so.
- `move` is a `Tasks` action (`id`, `board`) and a card menu item ("Move to board…" with the picker's list).
- **Built (B1a):** the source task closes as `done` with the result "Moved to work#12" and a `moved` history line, rather than a new `moved` state; a new state touches every column and filter, and Done is where §9.5 wants it shown. Media rows move to the new task (the file is by hash, so nothing is copied). `Tasks move` with a `board` moves between boards; with `rank` or `hold` it still reorders within a column.

### 9.6 Migration to boards

Covered with the rest of the migration in §8 (point 1): today's board becomes "Main", the default, with every number kept; `board_id` is added to the tables in the first migration of the build (B1a) and set to Main for every existing row; nothing is renumbered and no ref changes.

## 10. Offline and conflicts, concretely

| Situation | What happens |
|---|---|
| A machine is offline for minutes | Its board keeps working locally; a task homed elsewhere shows "home offline"; on return the layer catches both sides up by version vector |
| Two members edit different fields of one card | Both edits survive (per-field merge) |
| Two members edit the same field | The newer wins; the other value is in the card's history |
| Two machines file tasks at once | Different UUIDs, numbers from different blocks: no collision |
| A claim is made while its home is partitioned from the claimant | The claimant can't claim (no home); a claimant already holding a claim keeps the lease the home gave it; its completion applies if the fence still matches |
| A member is offline past the tombstone horizon (30 days) | It resyncs the board from scratch; its offline edits go to the conflict inbox |
| A machine's clock is far off | Its records are refused at the skew limit; the audit names it |
| A task is deleted on one member while another edits it | The tombstone wins over an older edit, loses to a newer one (the edit recreates content under the same id; the person sees "restored by X") |
| A machine is removed from the board | It keeps its copy, receives nothing after the key moves on, and its changes are refused |

## 11. AgentMux Cloud's part, and the console

The cloud tier is the layer's (§4.3): the board's records, sealed or readable according to the board's switch, stored and relayed by the hosted service. For the board:
- **Readable or sealed** (the layer's decision 2): a *readable* board lets the console and the mobile app show cards and notify; a *sealed* one stores records the service can't read, and those views show only that the board exists and its sync state.
- **What the console shows for a board** (Clamk's specification): members and their online state; tier and the cloud switch with a plain statement of what the service can read; sync health; the number of tasks, the media storage used and the cap; the conflict inbox; the audit rows; join approvals and removal. Every one of those is a read of the layer's collection views, so the console needs no Kanban-specific protocol.

## 12. Edge cases and tests

- **Unit (fake machines, in memory):** number blocks (exhaust, offline provisional ref, authority takeover); home-only lifecycle (a non-home write to a lifecycle field is refused); per-field merge and the history of an overwrite; a stale attempt after a partition; tombstone versus edit; import of two machines' boards with colliding numbers (`legacy_num`, `#5@Area54`, idempotent re-import); roster resolution (ambiguous slug); `scope: "anyone"` claim through a remote op; hand-off of an unclaimed task and refusal for a claimed one.
- **Integration (two or three real srv instances on one computer):** the whole path over `sync.*`: file on A for B's agent, B claims, A edits the title, B completes, A sees done; kill B mid-claim and watch the reaper reopen on its home; media fetch on demand.
- **Boards (B1, local):** per-board numbers (a bare `#n` is the default board's, `work#12` is Work's; the two numberings don't interfere); `Tasks` without `board` acts on the default board, the `list` hint line, `claim` across boards with targeted-first order; create, rename (slug unchanged), archive (read-only), delete (archived only, countdown, veto), set default; an agent that isn't on a board's agent list gets "unknown" for it; a container agent route; move (new number, marker, link back, refusal with claimed or `after`); migration of a pre-boards database to Main, idempotent.
- **The pane:** the picker (create, manage, archived group, counts), machine chips, the home line, the offline wait, the filter (component tests).
- **Live:** narko and one other host with the operator: join, import, file a task for an Area54 agent from narko, the numbers on both sides.

## 13. Build plan

One or two PRs each, with tests and a changeset. The layer's phases S0 to S4 are in its spec §12; the Kanban's part. **B1 needs no sync layer and ships first**, as the Vault's VS1 did.

| PR | Contents | Depends on |
|---|---|---|
| B1a | `db_boards` and `board_id` on the Kanban tables; the migration (today's board becomes "Main", the default, every number kept); per-board numbering and the `slug#n` ref grammar; the `board` parameter on `Tasks` (list, enqueue, claim, update…) and the `boards`, `board_create`, `board_edit`, `board_archive`, `board_unarchive`, `board_delete`, `board_default` and `move` actions, routes and MCP, including the container route; per-board agent lists enforced | none |
| B1b | The Kanban pane: the board picker (create, manage, archived group, per-board counts, the total "Needs you" badge), "Move to board…" on a card, copyable full refs | B1a, the UI kit's menu |
| K1 | The Kanban adapter (read a record, merge by §3.3's table, write back); the change log filled on every Kanban write; no network | layer S0, B1a |
| K2 | Number blocks and each board's numbering authority; provisional refs; the `home` field | K1 |
| K3 | Roster records; `target_agent` resolution across machines; `slug@machine` | K1, layer S1 |
| K4 | The remote lifecycle ops (`board.claim`, `heartbeat`, `complete`, `release`), the queued completion, `scope: "anyone"` | K2, K3 |
| K5 | Media as blobs, fetched on demand | layer S1 |
| K6 | Migration and merge of per-machine boards into Main (`legacy_num`, `#n@machine`) | K2 |
| K7 | The pane's machine chips, home line, machine filter, sync health | K3, B1b |
| K8 | Cloud tier for a board (the layer's S3), the readable/sealed switch per board | layer S3 |

## 14. For the operator

**Approved (2026-11-08):** the model of §1 (shared content, lifecycle at the task's home machine, numbers in blocks, today's boards merged by keeping narko's numbers), with several boards in the first build and a picker in the Kanban view, then "ok, greenlight kanban work". Still standing unless the operator says otherwise:
1. **Numbers per board** (§9.1): a bare `#168` is the default board "Main" and keeps working; other boards are `work#12`. Alternative: one global number across boards, which leaks activity across private boards and forces every board's machines to share one counter (not recommended).
2. **Cloud boards readable or sealed** (the layer's decision 2): recommended readable by default, with a per-board "private" switch.
3. **Several boards in the first build** (decided), shipped locally first (B1) and shared later over the layer.
4. **Untargeted tasks:** claimable only on the machine that filed them (today's behaviour, recommended), or by any member machine's agents by default.
5. **Auto-join:** same-account proven machines join a shared board without a prompt (recommended), as the fleet Global Memory does.
6. **New board defaults** (§9.2): "This machine only" preselected, sharing is a deliberate choice (recommended), or LAN-shared with every proven machine by default.
7. **`claim` without a board looks across every board the agent sees**, targeted tasks first (recommended), or only the default board.
8. **Deleting a board** by an agent shows the 15-second countdown with a veto and is audited, with no approval gate (recommended), per the rule that agents do anything except forge the operator's voice.
