# Spec: Memory after a compaction or a rebuilt session: an index of Personal Memory, nothing sent twice, nothing cut silently

Task #47923 (spec), #47924 (build). Author: Loap-3@narko, 2026-11-17.

**Status:** active (#7634 carries this document; the startup-file refresh in section 3.1 follows in a second PR).

Measured on one development host, AgentMux 0.60.32, from its srv logs, a screenshot of the pane list and one agent's own session start. No private memory text is quoted; fixtures in the build use made-up entries.

## 0. What the operator asked, and what is wrong

The operator, after a compaction: "shouldnt it only feed an index of memory, not the actual memories?" and "lets fully spec the memory issue, we should refine that, file and implement with recommendations".

What the code and the logs show:

1. **The Global Memory arrives twice and fills the delivery.** `compose_delivery` (`crates/srv/src/server/memory_delivery_handlers.rs`) decides `global_in_file` from the startup files, but it reads them only when the reason is `Startup` (`files` is empty for `Compact` and `Clear`). After a compaction Claude Code reloads `CLAUDE.md` by itself, and srv, knowing nothing of that, sends all Global Memory again. The same happens at a new session when the startup file is stale, because `global_memory_in_startup_files` needs every current section to appear byte for byte (`f.text.contains(&e.text)`).
2. **The startup file is stale because it is written only at launch.** `write_claude_md_respecting_ownership` is called from `agent_open.rs` and the editor's Launch path. Nothing rewrites it when Global Memory changes or when a compaction begins. One agent's `CLAUDE.md` was 13 days old and held an older form of an entry whose current form the hook carried. That is why the check failed and the second copy was sent.
3. **Personal Memory is last, so it is what is dropped.** The 8 hook parts of 9,000 characters (72,000) are filled by Global first. Personal files then come out as "not sent, 0 B", and, because the cut note sits after Global too, the list of names that `fit.rs` writes can itself be cut: one session start ended with a "memory cut here" note and no Personal Memory and no list of what was missing.
4. **The pane calls an index entry "not sent".** The card has two words for what is not in full ("cut", "not sent") and no word for "listed".

## 1. How memory reaches an agent today

Verified in the code unless marked.

| Path | When | What it carries | Global dedup | Personal |
|---|---|---|---|---|
| Startup file (`CLAUDE.md`, `.claude/AGENTMUX_MEMORY.md` by `@import`; `AGENTS.md`, `GEMINI.md`, `QWEN.md` for other providers; none for Kimi) | Written when the agent is launched (`agent_open.rs`, editor Launch). Claude Code reads it at session start and again after a compaction or `/clear` (observed in a compaction that delivered Global Memory twice; not documented by the provider) | All Global sections, one file | n/a | none |
| `SessionStart` hook, Claude Code only (`agentmux-bashwrap sessionstart --part 1..8`, `delivery.rs`) | `startup`, `clear`, `compact`. Not `resume` or `fork` (D10: the conversation is intact and the file is loaded) | Global (unless `global_in_file`), then every Personal file newest first, capped at 8,000 chars each, then a not-sent list, then the running summary after a compaction. Split in 8 parts of 9,000 chars; the parts arrive in the order the hooks finish | Only `Startup` (bug 1); needs a byte-exact match (bug 2) | Full text until the budget ends |
| Fallback re-delivery (`compose_fallback_with`, the frontend's hidden turn, `memory-reinjection-controller.ts`) | A compaction or fresh session where the hook stood down failed or does not exist: every provider but Claude | One message, no part cap, same content as the hook | Only `FreshSession` | Full text of every file, no budget |
| Catch-up turn (`compose_catch_up`) | A rebuilt session or a compaction: the fallback's memory plus the catch-up block | The fallback's text | As the fallback | As the fallback |
| Continuation record (`<agentmux-continuation>`, `continuity*.rs`) | A session AgentMux could not resume | AgentMux's record of the conversation and the running summary. **No memory**: its hidden memory turns are skipped by design | n/a | none |
| Claude Code's own `MEMORY.md` | Loaded by the CLI at session start. Whether it is loaded again after a compaction is **unverified** | The agent's hand-kept index | n/a | index |

Per provider: Claude Code uses the hook, so it has every path above. Codex, Gemini, Qwen, Aider, Goose, Pi and the other `AGENTS.md` providers have a startup file and the fallback, no hook. Kimi has neither a startup file nor a hook: only the fallback. Container agents: the hook goes through a container grant (`bind_to_grant`); their startup files are the mounted workspace's (`workspace_startup_files`); the Personal folder is on the host, so an index that names a host path is not readable from inside the container: it must name the `Memory` tool's read form (section 3.3).

The two memory systems are one folder for Claude agents: AgentMux's `Memory` tool, scope `personal`, and Claude Code's memory folder (`~/.claude/projects/<project>/memory/`) are the same files (`native_memory_handlers.rs` writes through to a durable mirror). Global Memory is a separate store, read by `Memory` scope `global`.

## 2. Measured sizes

Three agents on that host (anonymised), one day.

| Agent | Event | Startup file | Global in the hook | Personal | Result |
|---|---|---|---|---|---|
| A (Claude Code, host) | new session | about 62 KB, last written 13 days earlier | all sections: 72,000 chars arrived, about 11,000 more cut (about 83,000 in all, 37 entries) | 9 files, about 13 KB, not sent; no list of names arrived | Global Memory twice (once in the stale file); no Personal Memory; no names |
| B (Claude Code, host) | compaction | reloaded by the CLI | arrived again in the 8 parts | none | about 34,000 chars cut; parts arrived out of order (1, 2, 5, 3, 4, 7, 6, 8) |
| C (from a screenshot of the pane list) | rebuilt session | about 62 KB `CLAUDE.md` "+ Global Memory", about 16k tok | about 30 Global entries again; about 35k tok in all | 11 files "not sent, 0 B" | 5 workspace entries "not sent", 1 "cut", 17 cut in all |

Across every srv log still on disk on that host (versions 0.60.x): 325 of 325 compaction deliveries and 78 of 88 new-session deliveries used all 8 parts. A delivery of 8 parts is at or near the cap; this is a lower bound on how often a delivery is cut, not a count of cuts. The 10 that did not are probably sessions where the startup file matched (inferred from the part count, not checked per delivery), which is the case R1 makes the rule.

Cost of one delivery today: up to 72,000 characters, about 18,000 tokens (srv's own estimate is one token per four characters), on every compaction and most new sessions, of which about all is a copy of what the CLI already holds.

After the change, for this agent: an index of its 9 Personal files is about 1.8 KB (about 450 tokens: name, a description of 110 to 170 characters, type, one short line each) against the 12.7 KB of bodies, and the Global copy is gone. Estimated 2 to 8 KB per delivery, a 90% cut or more.

## 3. Design

The five accepted recommendations are kept. Where the code shows a better way it is said.

### 3.1 R1: never send what the startup file already carries, and keep it current

Two parts, both needed: the file must be current, and the check must be right.

**Make the file current (second PR).** The first PR ships the per-section check below and everything after it, so a stale file costs only the sections that changed. Keeping the files current needs one place where a Global Memory change commits, and there is none: entries are written by the RPC handlers, the `Memory` tool, revert, the fleet sync and the Operator Config seed. The follow-up therefore does not hook the writers: a watcher in srv compares a fingerprint of the Global sections every 20 s and refreshes the files when it changes. Not done at `PreCompact` or at the hook, on purpose: a refresh after the CLI has reloaded the old file would hide the changed sections from the delta and leave the agent with the old copy. One function, `refresh_startup_files(block)`, re-renders the agent's AgentMux-owned startup files (the same `render_agent_config_files` the launch uses, through the same ownership-aware writers, so a user-owned `CLAUDE.md` is never touched) and writes only `CLAUDE.md` and `.claude/AGENTMUX_MEMORY.md`, or the provider's `AGENTS.md` family where AgentMux owns it. Writes are atomic (temp file, rename). It runs:

1. When Global Memory changes, for every running agent on this host that has an AgentMux-owned startup file, debounced 3 s. (The build finds the one place a Global bundle change commits, and the fleet sync's apply path, and calls it from there.) This is the main fix: the file is current whenever the CLI reads it.
2. At `PreCompact`. The hook is synchronous: the CLI waits for its exit. The refresh runs in srv on the `compaction_started` event, and the hook does not return until srv has answered or 500 ms have passed (its existing timeout). This is the belt for the case where a change landed in the last 3 s, and it covers a container's workspace through the container route.
3. Not on `startup` (the CLI has read the file before the hook runs) and not on `resume` or `fork`.

**Make the check right.** Today the check is all or nothing and runs only at `Startup`. It becomes:

- Read the startup files for every reason (`Startup`, `Clear`, `Compact`) and for the fallback's compaction too. The card lists them only for `Startup`, as now.
- Compare per section, not for the whole block. A section is **carried** when the owned file contains its exact text. A section the file lacks or carries in an older form is **newer than the file**.
- Send only the sections newer than the file, under one heading, "These Global Memory entries are newer than the copy in your startup instructions", each whole. With the refresh above this is empty in practice; with a user-owned or missing file it is the right fallback and costs only what changed.
- When every section is carried, say so in one line (as `compose_items_with` does now) and send no Global text.

Refinement over the recommendation: R1 says "make the file current so that it is true". Current files alone are not enough, because a write can fail (read-only workspace, a foreign file, a container mount) and a user can edit the file. The per-section delta makes R1 hold even then, instead of falling back to the whole 72,000 characters.

### 3.2 R2: Personal Memory as an index, always complete

The delivery's Personal part is an index, never the bodies, except the "always" files of R3.

```
Your Personal Memory: 9 files in <folder>. Below, one line each. Read a file before you act on
what its line describes; the line is all you have until you do. Files marked ALWAYS follow in full.
- feedback-measure-before-claiming (feedback): <description>
- ...
Read one with the Memory tool: action read, scope personal, filename <name>.md (or open the path).
```

- Per file: its file name (and the frontmatter `name` when it differs), `type` (frontmatter `metadata.type`, else `unlisted`), and its description (frontmatter `description`, else the first non-heading line, 110 characters, as `fit.rs` does now). The folder is written once; each line carries the file name only, not a long absolute path.
- Both ways to read are given: the `Memory` tool's `read` for `personal` scope (works for container agents, whose host folder they cannot open) and the folder path (a host agent can `Read` it). A container agent's index gives only the tool form.
- The index is **complete by construction**: every file appears exactly once, in the index or, if marked ALWAYS, in full. It has its own budget of 8,000 characters and is its own part, so it fits one part whole whatever order the parts arrive in. If more than about 40 files push it over, descriptions shorten to 60 characters, then to the name alone; no file is ever left off. The `MEMORY.md` file is not listed as a file (the CLI's table of contents), and files that failed to read are listed as such, not skipped.
- Sorted newest first, ties by name. The order is for the reader; nothing depends on it.
- Written on every delivery that gives Personal Memory (`startup`, `clear`, `compact`, fallback), including `Startup`, so the agent has the same view every time.

**Refinement.** The CLI loads the agent's `MEMORY.md` index by itself, so one might skip the generated index. It is not skipped, for two reasons: whether `MEMORY.md` is loaded again after a compaction is unverified, and `MEMORY.md` is hand-kept and can be out of step with the folder (this agent's folder has two files it does not mention). The cost is about 2 KB. Decision D1.

**Memory-writing guidance.** The `Memory` tool's description and the Operator Config entry say to write a description that says when the file applies ("when X, do Y"), because the description is now what the agent sees. No format change; frontmatter stays as it is.

### 3.3 R3: "always send in full"

- **Marking.** A file is marked by `deliver: always` in its frontmatter (found at any indent, as `frontmatter_value` already finds `type`, so `metadata:\n  deliver: always` works). No new file or RPC: it is a line the agent or the person writes with the `Memory` tool or the Memory pane.
- **Who.** Anyone who can write the file: the agent for its own memory, the person in the Memory pane. No operator-only gate (workspace rule). Another agent cannot write someone's Personal Memory, so nothing changes there.
- **Limit.** At most 5 files and 6,000 characters in all. A file that alone is longer than 6,000 characters cannot fit, so it goes in the index with the reason; there is no per-file cut any more (the old 8,000-character cut applied to bodies, and no body rides now except these). The limit is applied by explicit order, never by position in the delivery: marked files are admitted in the order of their `modified` time, oldest marking first, until a limit is reached. A marked file over the limit is not dropped: it goes in the index with the words "marked ALWAYS but over the limit (5 files, 6,000 characters); read it". The agent is told the limit in the Memory tool's description so it can trim.
- The pane shows an ALWAYS file as "in full" with a pin mark, and an over-limit one as "in index" with the reason.

### 3.4 R4: the pane's "Given to the agent" list tells the truth

Each row is in exactly one state, in plain words:

| State | Row chip | Tooltip |
|---|---|---|
| Sent in full | none (as now), pin for ALWAYS | "Given to the agent in full." |
| In the startup file | "in startup file" (exists as `via: startup_file`) | "Already in your startup instructions, so it was not repeated." |
| Listed in the index | "in index" (muted) | "Listed by name and description. The agent reads the file when it is relevant." |
| Cut | "cut" (warning) | "Only part fit. The delivery says how much is left and where to read it." |
| Left out | "left out" (error) | "Not given: <reason>. Its name and how to read it were in the delivery." |

"not sent" goes away. The title becomes `Given to the agent · new session · 53 items · 3 in full · 11 in index` and adds `· N left out` or `· N cut` only when above zero. The size column for an index row shows the size of its index line, not "0 B". Backend: `EntrySize.delivered` gains `indexed` and `startup`; `omitted` keeps its meaning of left out and now always carries a `reason`. A client older than this change reads an unknown `delivered` value as a full row, which is the right side to err on: it never says "not sent" for a file that was listed.

Screenshots: before is the operator's screenshot of a rebuilt session's list (53 items, 17 cut, 11 Personal files "not sent"). After is taken in the build from my own task dev, with a made-up agent, the same list, and added to the PR. macOS and Linux look checks follow the visual-change rule (Clare, Maricon).

### 3.5 R5: nothing is dropped by position

The composer builds the delivery from classes, each with its own reserved budget, in this priority order. A class never takes another's room.

1. Header and the index (8,000 characters, its own part).
2. ALWAYS files (6,000).
3. Global sections newer than the startup file (the rest of the delivery's room, at most 72,000 less the above).
4. The running summary, after a compaction (reserved, as now).
5. The left-out note, and the pruning note.

If something does not fit, **whole entries** are left out, never the end of the text, and the left-out note names each one with the way to read it (`Memory read global <entry>` or `Memory read personal <file>`), up to the full list (no 25-name limit: a name is about 60 characters, so even 100 are 6 KB, inside the note's reserved room; past that the note names the first 100 and says "and N more: `Memory list`"). The note is composed before the split and reserved, so it cannot be the part that is cut. The final cut note ("memory cut here") stays only as a last guard and a test fails if it ever appears in a delivery built from these classes.

The fallback has no part cap, so it carries the same content with the same classes: no 9-part wall of text, no Personal bodies.

### 3.6 Path by path

- **Startup file:** kept current (3.1). No Personal Memory in it.
- **SessionStart hook:** 3.1 check on all three reasons; index, ALWAYS, delta, summary, notes. The number of parts used falls to about 1 to 3 of 8; the unused hook commands answer `of: 0`, as they do now.
- **Fallback and catch-up:** same composer (`compose_fallback_with` takes the same inputs and, now, the startup-file check for compaction as well as fresh session).
- **Continuation record:** unchanged: it carries no memory; the memory rides in the hidden turn that follows it.
- **Resume and fork:** no delivery (D10). The refresh keeps the file current, so a resumed session reads current Global Memory on its next compaction.
- **Codex and other `AGENTS.md` providers:** the same check against their startup file, through the fallback. Kimi has no startup file, so its Global Memory is always carried by the fallback in full; its Personal Memory is an index. Container agents: index in the `Memory` read form; the startup check reads the mounted workspace only.

## 4. The risk of an index: the agent that never opens the file

An index helps only if the agent opens a file when it matters. Three measures:

**Wording.** The index opens with a sentence that makes opening the default, not an option: "Read a file before you act on what its line describes; the line is all you have until you do." Each line is the file's description, which the memory-writing guidance asks to say when the file applies. ALWAYS (3.3) is the safety net for rules that must hold every time, so the index never has to be the only carrier of a rule that cannot be missed.

**A calibrated check (a lab: not run until the operator says yes through AgentY).**

- Question: when the memory is an index, does an agent open and use the file it needs, as often as when it is given the text?
- Fixture: a throwaway agent on my own task dev, with made-up memory: 12 files, of which 3 each hold one fact that is needed for a task (for example a made-up name for a made-up build slot) and 9 are unrelated. No real memory text.
- Arms, same 3 tasks, same model, 10 runs each, in random order:
  - **Control:** full text of all 12 (today).
  - **Treatment:** the index as in 3.2.
  - **Sensitivity:** the index with the three relevant descriptions replaced by vague ones ("notes"). This arm must fail if the check can see anything; if it does not drop, the check is blind and its result means nothing.
  - **Floor:** nothing delivered (shows the agent cannot guess the fact).
- Measure per run: whether the right file was opened before the answer, whether the answer is right, tokens of the delivery.
- Pass: treatment correct at least 90% and within 10 points of control; sensitivity at least 30 points below treatment; floor near 0.
- Cost: 40 runs of a short task, about an hour on narko or Area54, no real mouse or screen use, no network. I will ask AgentY for the operator's yes before running it, as the rules require; the build does not wait for it, because the delivery path is covered by tests and the operator checks a real compaction before anyone calls it fixed.

**Operator check.** The operator watches one real compaction in the release that carries this, as AgentY asked.

## 5. Guards (tests that fail)

Backend, next to the code (`delivery.rs`, `fit.rs`, the handlers' `tests.rs`, `startup_files`):

1. **Never repeat the startup file.** For each of `Startup`, `Clear`, `Compact`, the fallback's `FreshSession` and `Compaction`: a fixture agent whose owned startup file carries every Global section gets a delivery with none of those sections' text in it, and the one-line "already in your startup instructions".
2. **Delta only.** The file carries 36 of 37 sections, the 37th in an older form: the delivery carries the 37th, whole, and none of the other 36.
3. **The index is complete.** For generated sets of 0 to 200 Personal files with random descriptions (missing, long, with quotes), every file name appears exactly once in the index or in an ALWAYS block; the index is at most 8,000 characters and sits inside one part; a file with unreadable text is named as unreadable.
4. **Nothing is cut without being named.** For Global sets of 20 to 120 KB and Personal sets as above: the parts never end with the "memory cut here" note; if any whole entry is left out, the left-out note names every one of them with its read form. A fuzz over sizes fails if an entry is neither in the delivery nor in the note.
5. **ALWAYS limits.** Six marked files, or marked files over 6,000 characters: the extras are in the index with the over-limit reason, and none vanishes.
6. **Part order.** Any permutation of the parts of a delivery rebuilds the same content (each part labelled, the index whole in one).
7. **Refresh.** A Global bundle change rewrites an AgentMux-owned startup file of a running fixture agent and never a user-owned one; `PreCompact` rewrites it; a read-only file reports and the delta of 3.1 still holds.
8. **Containers.** The container fixture gets an index with the tool form only, and its startup check reads the mounted workspace only.

Frontend (`context-delivery.test.ts`): each state shows its chip, title counts and tooltip; no row says "not sent"; an unknown `delivered` value from an older srv falls back to the old words. The i18n ratchet covers the new strings.

## 6. Build plan

One PR if it stays reviewable; if the diff passes about 1,500 lines, two: (1) server: the check, the refresh, the composer and R5, with tests; (2) the index, ALWAYS and the pane wording. Changes:

- `crates/srv-core/src/backend/memory/`: `delivery.rs` (composer classes, index, delta), `fit.rs` (budgets by class, left-out note), a new `index.rs` for the index line, ALWAYS parsing and the part placement.
- `crates/srv/src/server/memory_delivery_handlers.rs`: the startup check for every reason and the fallback's compaction; `EntrySize` states; the refresh call at `compaction_started` (host and container routes).
- `crates/srv-core/src/backend/startup_files.rs` / `agent_config.rs`: `refresh_startup_files` and its call from the Global bundle commit and the fleet sync apply.
- `frontend/app/view/agent/context-delivery.ts`, `components/ContextDeliveryCard.tsx`: wording and chips.
- The `Memory` tool's description (`crates/mcp`): the ALWAYS mark, its limit, and the guidance on descriptions.
- A changeset, tests, before and after screenshots.

Out of scope, and said so: **Global Memory itself.** It is about 62 to 83 KB (roughly 16,000 to 21,000 tokens) in every agent's startup context, which is far more than the Personal Memory this spec moves to an index. Trimming or indexing the Global entries is a different decision for the operator and AgentY; this spec only stops it being sent twice and being what pushes everything else out.

## 7. Decisions for the operator

Each has a recommendation; the build follows them unless AgentY says otherwise.

- **D1. Index always, or skip files `MEMORY.md` already lists?** Recommend always: about 2 KB, and it does not depend on whether the CLI reloads `MEMORY.md` after a compaction (unverified) or on `MEMORY.md` being complete.
- **D2. ALWAYS limit and mark.** Recommend `deliver: always` in frontmatter, 5 files and 6,000 characters, marked by the agent or the person, over-limit files demoted to the index with the reason (never dropped).
- **D3. A stale startup file: resend the Global Memory, or only what changed?** Recommend only what changed, whole sections, under one heading.
- **D4. Rewrite the startup files of running agents when Global Memory changes?** Recommend yes, debounced 3 s, owned files only. It writes a file in each agent's workspace the agent did not ask for; the file is AgentMux's own and the CLI reads it only at start and after a compaction.
- **D5. Providers with no startup file (Kimi).** Recommend leaving Global Memory in the fallback in full for now, and the Personal index as everywhere.
- **D6. Run the calibrated check (section 4)?** Recommend yes, after the build, about one hour. Needs the operator's yes through AgentY.
- **D7. Global Memory size.** Not part of this build. Recommend a separate look: it is the larger share of every agent's startup context.

## 8. What I could not verify

- That Claude Code rereads `CLAUDE.md` after a compaction before the `SessionStart` hook runs (a compaction observed to deliver Global Memory twice shows it rereads it; the order is not documented). The refresh at `PreCompact` precedes either order.
- That `MEMORY.md` is reloaded after a compaction.
- The 325 and 88 delivery counts are from the srv logs still on one host, not from every host.
