# SPEC: Shared Kanban: one board across a user's machines, by LAN or AgentMux Cloud

**Status:** draft, 2026-11-01. Spec task #17016, Naki-2. Not yet approved by the operator.
**Author:** Naki-2@narko.
**Trigger (operator, 2026-11-01):** "I just have 2 ideas: 1) Managed vault 2) Managed Kanban .. in both cases, users of AgentMux Cloud can use kanbans and vaults from their own cloud", then "users should still have this ability to shared via the LAN tier, without cloud.", then "the other question of whether users can have multiple vaults/kanbans", then "their own cloud means an agentmux hosted service", and on names "Kanban and Vault will simply retain their names".
**Verified on:** `main` at `e4f374c61` (2026-11-01), AgentMux 0.60.10.
**Depends on:** `SPEC_USER_DATA_SYNC_LAYER_2026_11_01.md` (the sync layer both the Kanban and the Vault use; "the layer" below). **Extends:** `SPEC_WARDEN_KANBAN_2026_10_20.md` (§6.11 says the board is per machine and "other computers … are Phase 4"; this is that phase) and `SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` (§4.3, `tasks.enqueue` and `tasks.list` with `host`).

## 1. Summary

Today the Kanban is per machine. A task for an agent on another machine goes by message, or by `Tasks enqueue` with `host` onto that machine's own board, and the same `#number` means different tasks on different machines. This spec makes **one board a user's machines share**, in the three tiers of the layer:
1. **Local**: as today, one machine.
2. **LAN**: a user's machines share the board peer to peer, with no cloud.
3. **AgentMux Cloud**: the board also syncs through the hosted service, for machines that are never on one network.

The design in one paragraph: **content is shared by everyone, lifecycle belongs to one machine.** Any member edits a task's title, notes, links and order, and the layer merges those field by field. A task's *lifecycle* (who claimed it, its state, its attempts and lease) is decided only by the task's **home machine**, the one where its target agent lives, so a claim never needs two machines to agree and today's lease, attempt fence and reaper carry over unchanged. Numbers stay unique because each machine leases blocks of them from the board's numbering authority. Today's per-machine boards merge without renumbering what people refer to.

## 2. What exists (verified in code)

- **Schema** (identity store, per machine): `db_work_queue` (`id` UUID, `title`, `payload`, `kind`, `target_agent`, `target_group`, `state` open | held | claimed | blocked | review | done | failed | cancelled, `claimed_by`, `claimed_by_uid`, `claim_expires`, `attempts`, `max_attempts`, `created_by`, `created_by_uid`, `not_before`, `result`); `db_work_cards` (`num` INTEGER PRIMARY KEY AUTOINCREMENT, `work_id` UNIQUE, `repo`, `branch`, `pr_url`, `rank` REAL, `review_required`, `checklist`, `question`, `note`, `requested_by`, `requested_by_kind`, `request_text`, `filed_by`); `db_work_events` (append-only history, `actor`, `actor_kind`, `kind`, `from_state`, `to_state`, `body`); `db_work_deps` (`work_id`, `after_id`; at most 10 per task).
- **Numbers:** `num` is per machine; the `Tasks` tool takes a UUID or `#num` (`id_or_num`). People and messages say `#168`. There is no `host` column.
- **Claims:** an atomic claim sets `state='claimed'`, `claimed_by`, `claimed_by_uid`, `claim_expires = now + lease` (default 2 minutes) and `attempts += 1`; heartbeat extends the lease; a reaper reopens an expired claim (or fails it at `max_attempts`); every heartbeat, complete and release carries the **attempt** from the claim (an ABA fence) so a stale call can't touch a newer claim. The holder is matched by agent uid.
- **Across machines today:** `tasks.enqueue` and `tasks.list` as remote ops (`work_remote.rs`): the target is named by slug and resolved on the owning machine (`resolve_typed_name`; ambiguous or missing is refused with candidates); the task is recorded as filed by `slug@machine` (kind `agent`), never the operator; no `target_group`.
- **The pane** shows agent name, provider icon and state; **no host or machine appears** in the task schema or the card (§6.11 of the Kanban spec says other computers are Phase 4).
- **The queue is shared by every AgentMux on one computer** (the identity store, `work_queue.rs`): a per-channel board was rejected on purpose. The shared board keeps that.

## 3. Model

### 3.1 One board is one collection

The board is a layer collection of kind `kanban` (id, name "Main", tier, members, key). Every table above gains a `collection_id` (default: the one board), so room for several boards costs nothing now (§9).

### 3.2 Task numbers that stay unique across machines

Today's `num` comes from one machine's autoincrement. On a shared board:
- **The numbering authority** is one member (the machine that created the board; today that is narko). It owns the counter.
- **Blocks:** every machine leases a block of 50 numbers from the authority when it joins and again when its block runs below 10. A task gets its number from the machine's block when it is filed, and keeps it forever. Numbers are unique, increase on each machine, and are not in time order across machines (a gap or an out-of-order `#` is normal).
- **Offline with no block left:** the task is filed with a provisional ref, `#pending-<machine>-<n>`, usable in every `Tasks` call on that machine, and receives its real number the moment the machine reaches the authority. The provisional ref keeps working as an alias.
- **If the authority is unreachable for good,** any member may take over the counter after the person approves in the host window (the next number is above every one it has seen).
- **Reading a number:** `#n` means the board's task `n`. On a board other than the default (§9) it is `board#n`, and `#n` alone means the agent's own default board.

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
- A target that doesn't exist yet on any member is refused at filing, as on one machine today.

### 4.4 Home hand-off

If a home machine stays offline beyond a limit the person sets (default 12 hours), a member may **take over the home** of that machine's *unclaimed* tasks, after the person approves in the host window; the home returns on its next contact and the board shows the tasks' history. A *claimed* task is never taken over: its lease lapses and the reaper reopens it, which is the existing rule.

## 5. Media on cards

Today a card's media are files in the filestore, served by the machine. On the shared board a card's media are layer **blobs**: content-addressed chunks (the layer §3.4), referenced from the card by hash, name, type and size. A machine fetches a card's media on first view from any member that has them (LAN), or from the hosted service (cloud tier), and keeps them; a card shows a placeholder until then. The size cap per file is the collection's (default 25 MiB). A deleted card's media are removed on every member when the tombstone is acknowledged.

## 6. Dependencies, holds, review, the operator's columns

- **Spec and build pairs** (`first: "spec"`) are two tasks with an `after` edge, both homed by their targets. A build whose spec is homed on another machine starts when the home machine of the build sees the spec `done` in its replica; the replica lags the spec's home by seconds on a LAN, so the build may start slightly late, never early (a spec still replicating as `review` is not `done`).
- **Holds** (`hold`, `unhold`) are lifecycle: any agent or the operator asks, and the home machine decides, with no attempt consumed (as today).
- **Review** (`review_required`, accept, send-back): the requirement is content; accepting and sending back are lifecycle, done by the operator from any member's pane as a remote op to the home machine.
- **The operator's columns** (Backlog, Doing, Review, Done…) are derived from state and `after`, as today; the same board shows on every member, with each card's home on it (§7).
- **Done and failed followers:** failed predecessors still hold followers (today's rule), on every machine.

## 7. The pane: which machine each agent is on

- Each agent chip shows `name · machine`, with the machine's state dot (online, offline since …, never synced).
- A card shows its home machine when it isn't the viewing machine, and "waiting for Area54 (offline since 10:12)" for a task whose home is offline.
- A **machine filter** next to the existing filters; a "this machine only" view for the operator who wants today's picture.
- The board's header shows the tier and sync health (in sync, N changes waiting, last contact) with a link to the collection's console view.
- Swarm already shows remote agents with a platform tag; the board reuses that roster rather than a second source.

## 8. Migration: merging today's per-machine boards without renumbering what people refer to

**What people refer to is the narko board:** `#168`, `#16989`, `#17016` are the operator's and the leaders' numbers. Other machines' boards (Area54, starpower, charlie) are small and used by their stewards, and their numbers collide with narko's (every `#5` exists on several machines).

1. **The first machine to enable the shared board becomes the numbering authority and keeps every number it has.** Its tasks join the shared board unchanged: UUIDs and numbers as they are.
2. **Every other machine's tasks are imported once**, when that machine joins:
   - **Open, held, claimed, blocked and review tasks** get a new number from that machine's first block; the old local number is kept as `legacy_num` with the machine, and `#<old>@<machine>` (for example `#5@Area54`) resolves to the task for good. On the importing machine a bare `#5` typed by an agent that knows only the old number resolves through `legacy_num` for 90 days, with a note in the answer ("this is now #17042").
   - **Done, failed and cancelled tasks** are imported as history with their old number shown as `Area54 #5` (no new number is spent on them) and the same `#5@Area54` ref.
3. **Dependencies and links** between a machine's own tasks are rewritten to UUIDs (they already are in `db_work_deps`); a PR or task number written inside a task's text is left as written (it is the author's words).
4. **No task is lost or duplicated:** the import is keyed by the UUID (`id` is already unique), so re-running it, or two machines importing at once, is harmless.
5. **Mixed versions on one computer** (the Kanban spec §10): an older build on the same machine doesn't know the shared board and sees only the local tables; it keeps working on its own tasks and never sees remote ones. The migration doesn't remove anything an older build reads.
6. **`Tasks enqueue host:` and `tasks.list host:`** keep working; on a shared board `host:` is sugar for a target on that machine (§4.3).

## 9. Several boards per user (the operator's question)

**What it would be:** boards "main", "work", "this project", "team", each its own layer collection with its own members, tier and key (this machine only; the LAN machines; cloud; other people later). **The first build ships one board per user and leaves room**: `collection_id` is on every row and every layer message, the `Tasks` tool takes an optional `board`, and nothing in the schema assumes one.

Why not several now: three things change meaning and each needs a decision with the operator's habits in it.
- **What `#168` means.** With one numbering authority per board, `#168` is ambiguous across boards. The proposal: `#n` is the agent's default board; `board#n` names another (§3.2). That changes how every agent writes a ref.
- **Which agents see which board.** Proposal: an agent sees a board if it runs on a member machine and the board's agent list (default: all agents) includes it; a container agent sees boards through its host machine's membership. A restricted board (an agent list of two) is how a team or a client project would stay apart.
- **Moving a task between boards:** a new task on the target board, a "moved to work#12" marker and link back on the source (the layer §9: no cross-collection identity), the history kept on both.

A person who wants a second board in the first build can have it from the same code; the cost is those three decisions, not the sync.

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
- **The pane:** machine chips, the home line, the offline wait, the filter (component tests).
- **Live:** narko and one other host with the operator: join, import, file a task for an Area54 agent from narko, the numbers on both sides.

## 13. Build plan

One or two PRs each, with tests and a changeset. The layer's phases S0 to S4 are in its spec §12; the Kanban's part:

| PR | Contents | Depends on |
|---|---|---|
| K1 | `collection_id` on the Kanban tables; the Kanban adapter (read a record, merge by §3.3's table, write back); the change log filled on every Kanban write; no network | layer S0 |
| K2 | Number blocks and the numbering authority; provisional refs; the `home` field | K1 |
| K3 | Roster records; `target_agent` resolution across machines; `slug@machine` | K1, layer S1 |
| K4 | The remote lifecycle ops (`board.claim`, `heartbeat`, `complete`, `release`), the queued completion, `scope: "anyone"` | K2, K3 |
| K5 | Media as blobs, fetched on demand | layer S1 |
| K6 | Migration and merge of per-machine boards (`legacy_num`, `#n@machine`) | K2 |
| K7 | The pane: machine chips, home line, filter, sync health | K3 |
| K8 | Cloud tier for the board (the layer's S3), the readable/sealed switch | layer S3 |
| K9 | Several boards (§9), if the operator wants them | K1, layer S4 |

## 14. For the operator

1. **Approve the model** (§1): shared content, lifecycle at the task's home machine, numbers in blocks, today's boards merged by keeping narko's numbers.
2. **Readable or sealed cloud boards** (the layer's decision 2): recommended readable by default with a per-board "private" switch.
3. **One board in the first build with room for more** (recommended) or several now (§9).
4. **Untargeted tasks:** claimable only on the machine that filed them (today's behaviour, recommended), or by any member machine's agents by default.
5. **Auto-join:** same-account proven machines join the board without a prompt (recommended), as the fleet Global Memory does.
