# Spec: Kanban tasks carry media; a UI change that needs the operator's OK comes with screenshots or a recording

**Status:** approved by the operator with the recommendations D1 to D5 of §10 (2026-10-25, through AgentY), plus: phase 2 (recording) starts right after phase 1. Phase 1 is built in #4285.
**Date:** 2026-10-25
**Author:** Naki-2@narko
**Task:** Spec #4284; the build is #4285 (after this is approved)
**Requested by:** the operator, through AgentY. Their words: "tasks with UI reviews, that change the UI and require a human OK, those should come with media (images/recordings)."
**Verified against:** `agentmux` code read in my clone (main of 2026-10-25); the storage, tool, route, card and drawer code named below was read, nothing here has been run.
**Builds on:** [SPEC_WARDEN_KANBAN_2026_10_20.md](SPEC_WARDEN_KANBAN_2026_10_20.md) (the board, the card, the drawer), [SPEC_KANBAN_ACCEPT_ON_THE_OPERATORS_WORD_2026_10_23.md](SPEC_KANBAN_ACCEPT_ON_THE_OPERATORS_WORD_2026_10_23.md) (who may act on the board)

## 1. Ask

1. A task can carry **files**: images (PNG, JPEG, GIF, WebP, SVG) and recordings (MP4, WebM, MOV), attached by the agent that works on it through the `Tasks` tool.
2. The card and the drawer **show them**: thumbnails, a poster for a recording, a click opens the file in the Editor, with who attached it, when, and on which attempt.
3. **The rule:** a task whose work changes the UI and needs the operator's OK comes with media when it is handed in for review.
4. Agents have a way to **capture** that media from their own test build, and know what to check before attaching it.

## 2. What is there today

- **Tasks carry no files.** A task is a row in `db_work_queue` plus one row in `db_work_cards` (`storage/work_board.rs`: `repo`, `branch`, `pr_url`, `checklist`, `question`, `note`, provenance), history in `db_work_events`, the spec or report path in `db_work_docs` (`storage/work_prs.rs`). The agent-facing `links` is `{pr, branch, doc}` only; `doc` must be an absolute path and is shown in the drawer under "Document" with an Open button that opens it in the Editor (`kanban-drawer.tsx`).
- **The `Tasks` tool** (`crates/mcp/src/tool_schemas.rs` `TASKS_TOOL`, `tools/tasks_tool.rs`, `tools/work.rs`) has no file parameter. `enqueue` takes no `links`.
- **Attempts** are a counter (`attempts`) on the queue row that also fences every mutating call (`attempt`); nothing records what happened in which attempt except `db_work_events`.
- **A task is never deleted.** Cancel is a state (`work_queue_cancel`); nothing removes queue or card rows. Only side tables are pruned: events of finished tasks 30 days after their last update, and PR/doc rows whose task is gone (`run_reminder_loop`, hourly).
- **Message `send` files** is the pattern for files in this codebase: the MCP reads each path, checks it (readable, non-empty, at most 10 files, 100 MiB per file and 200 MiB per message on this machine, less for LAN and WAN), uploads the bytes to `POST /api/v1/attachments/upload`, and the jekt carries a manifest of `{sha256, name, size}`. The receiver sees absolute paths in an `<attached_files>` block. The store (`<data dir>/attachments`: content-addressed blobs, 256 px thumbnails for images) is general, but it **sweeps** files by last use (30 days; 7 for a file never sent; at most 128 files), so it is not a home for something the operator may look at later.
- **Screenshots exist.** `UI` action `screenshot` captures one of the caller's own panes as a PNG (CDP `Page.captureScreenshot` of the pane's clip), blacks out secret fields (`screenshot_redact`), and writes `<data dir>/tmp/ui-screenshots/<uuid>.png`, deleted after an hour. `UI` action `window` captures another OS window of the machine (PNG, with a Vault-on-screen check, an audit log and a capture tier).
- **There is no screen recording** anywhere: no recorder, no encoder, no frame capture in the repo (the only recording code is voice dictation).
- **Showing media exists.** `GET /agentmux/stream-local-file?path=` streams a local file (up to 500 MB, ranges). `frontend/app/element/local-media.ts` fetches it with the window's credentials into a blob URL, reads a byte range for a recording's poster frame (the first 2 MiB), and caps inline images at 25 MiB and recordings at 200 MiB. The Editor opens images, video and audio.
- **No poster generator.** A recording's poster comes from the browser reading the first 2 MiB; a file whose index (moov atom) is at the end gets a placeholder.

## 3. Design: attaching files

### 3.1 The tool

`Tasks` takes **`files`**: an array of absolute paths, on `enqueue`, `update` and `submit` (and `complete`, which is `submit` for a task that needs no review). Same checks as `Message send`'s `files` run before anything is sent (a readable, non-empty file, resolved from the agent's working directory). Each file is also given an optional caption through the paired array `captions` (same order; short text, "after: the Kanban in narrow width"). The answer lists what was attached (name, kind, size) and what was refused and why.

**Upload, then reference** (the Message pattern), so a call is atomic: the MCP uploads each file to a new `POST /agentmux/work/media` (behind the auth key, like the attachments upload: the agent-token layer buffers whole bodies, so a streamed upload is not behind it; the task call that names the file is), which streams it to a staging folder, hashes it and sniffs its type; the task call then carries `media: [{sha256, name, caption}]`. A bad file fails the call before the task changes. Staging files left behind are removed after an hour.

### 3.2 What is accepted

By **sniffed content**, never by the name or the extension alone:

| Kind | Types | Largest |
|---|---|---|
| Image | PNG, JPEG, GIF, WebP, SVG | 25 MiB |
| Recording | MP4, WebM, MOV | 100 MiB |

- At most **10 files per call**, **30 per task**, **300 MiB per task**. A call over a limit is refused whole, with the limit named.
- An **SVG** is shown only through an `<img>` (blob URL), never inlined, so its scripts never run; it is the same path inline images in the agent pane already take.
- Anything else (PDF, archives, executables) is refused: the Kanban is not a file store; a spec or report still goes through `links.doc`.

### 3.3 Where they live

- **A durable store owned by srv,** `task-media/`, **in the shared home** beside the store that holds the tasks (`~/.agentmux/shared/`), not in a channel's data dir: the tasks are shared by every channel, so their files must be too, or a second channel would show a card whose picture is missing. Content-addressed: `task-media/<aa>/<sha256>.<ext>`, thumbnails beside it. The same bytes attached to two tasks are stored once.
- **The Kanban's own table,** `db_work_media`, created with `CREATE TABLE IF NOT EXISTS` like the other board tables (no schema-version bump, so an older build sharing the store never sees it): `work_id`, `sha256`, `name`, `kind`, `size`, `caption`, `attached_by` (agent name and uid), `attempt`, `at`, `removed_at`.
- **It does not use the attachments store,** because that store's sweep (30 days, 128 files) would delete a picture the operator has not looked at yet. It reuses that store's image code (decoding guards and the 256 px thumbnail) rather than copying it.
- **Removal.** A task is never deleted today, so "removed when the task is deleted" has nothing to hook to. Recommended (D2): the media of a task are removed **30 days after it finishes** (done, cancelled or failed), by the same hourly prune that clears its events, and a blob is deleted when no row names it. The operator can also remove one file from the drawer at any time (§4.3).
- **An attached file is a copy.** The agent's own file can be cleaned up (build-cache cleanup, workspace tidying) with no effect on the card.

## 4. Design: the card and the drawer

### 4.1 Card

A small strip under the title of up to three thumbnails (image thumbnails; a recording shows its poster with a play mark), with "+N" for the rest. A task with none shows nothing, so the board does not change for the tasks that have no media. A task that is marked as a UI change and has none shows a quiet "no media" pill (§5). `kanban.list` carries only `media_count` and the first three file ids, not the whole list.

### 4.2 Drawer

A **Media** section above Document: a grid of thumbnails, newest attempt first, each with its caption, who attached it, when and on which attempt ("Naki-2, attempt 2, 14:05"). A click opens the file in the Editor (`createBlock({ view: "editor", file: <stored path> })`, as Document does today); a recording also plays inline in the drawer, muted, with controls, started by a click and never on its own (the rule the agent pane already follows). Loading goes through `local-media.ts` (credentials, blob URL, poster by range), so no new transport is built for the page. The drawer's media come from `kanban.task`, which returns each file's stored path, so the Editor has something to open.

### 4.3 Who changes them

Agents add; **only the operator removes** (`kanban.act` `media_remove`, operator-only through the host-proven connection like `accept`). An agent cannot delete another agent's picture, and the operator can take out a file that shows something it should not. A removed file leaves a row marked `removed_at` so the drawer says "removed" and who asked.

## 5. The rule

**When a task whose work changes the UI and needs the operator's OK is handed in, it carries media.**

- **How "changes the UI" is known (D1):** a **flag on the task, `ui_change`**, boolean, set by whoever files or works it: `Tasks enqueue` takes `ui_change: true` (a leader filing "fix the Kanban's narrow view" sets it), and `update` and `submit` take it too, so the agent that actually did the work can set it when the task turns out to touch the UI. The card has a toggle for the operator. A flag, not a `kind` (kind is already the spec/build/research tag, and a build can be UI or not) and not "the PR touches frontend files" (that needs the PR's file list from GitHub, whose anonymous budget the Kanban poller already spends; and a backend task can still change what the operator sees).
- **Needs the operator's OK** is what `review_required` already means (a task the operator asked for waits for their review).
- **Submit refuses, with a way out (D3).** `submit` of a task with `ui_change` and `review_required` and no media is refused with: "This task changes the UI and waits for the operator's OK: attach a before/after screenshot, or a short recording for motion or an interaction (`files`), captured from your own test build. If you cannot capture one, say why in `no_media_reason`." A `no_media_reason` lets it through and shows on the card ("No media: <reason>") so the operator sees it was a choice, not an oversight. Refusing is recommended over a warning: the rule exists because a warning is the thing that gets skipped. The reason is the escape for the macOS-less agent, the headless host, and the change that has nothing to see.
- **When a PR is linked** (`update` with `links.pr` on a task with `ui_change` and no media), the answer carries a reminder, not a refusal, because that is mid-work.
- **What counts as media:** at least one image or recording attached in **this** attempt. Media from an earlier attempt shows in the drawer but does not satisfy a new submit after the operator sent the task back.
- **What it does not do:** it does not judge the picture. A leader accepting or the operator reviewing sees it; the check is that something was attached on purpose.
- **A task not marked** (`ui_change` false) is unchanged: media may still be attached to any task, and shows the same way.

## 6. How agents capture media

What exists and what is added, per kind:

- **Screenshots (exists).** `UI` action `screenshot` of a pane in the agent's **own isolated test build** (`task dev`): a before from the base, an after from the change, same pane and size. The result's path goes straight into `files`. For a window, `UI` action `window` on the test build's window, found by the process id the agent started itself.
- **A recording of motion or an interaction (does not exist).** Phase 2 (§9): a `UI` action `record` (`start` on a pane of the agent's own test build, `stop` returns a file) that collects frames from the page (CDP screencast) and encodes a short **animated WebP** or **GIF** with a Rust encoder already in the tree or a small one added. It is limited to the caller's own pane (as `screenshot` is), to 30 seconds and 10 frames a second, redacts secret fields on every frame the way `screenshot` does, and refuses a pane that is not the caller's. This is not verified: I have not run CDP screencast in the CEF host on any OS, and whether the engine's build allows it, or whether the encode time is acceptable on each OS, is the first thing the build checks (and if it does not hold, phase 2 is reported back instead of built).
- **Until phase 2:** a short recording is an optional extra; the rule is satisfied by a **filmstrip** of screenshots (one per step, captioned), which already covers "motion" for most UI changes (a drawer opening, a menu, a resize). An agent with a screen recorder of its own may attach an MP4/WebM it made; nothing here depends on that.
- **Never from the operator's instance.** Capture comes from the test build the agent started itself. The tools already scope `screenshot` to the caller's own pane; `window` on a window the agent did not start is not what this asks for, and the rule text says so.

## 7. Privacy

Media can show secrets, other people's work or other agents' panes. What the capturing agent must do:

1. Capture only its **own test build** (its own pane or window), never the operator's running AgentMux or any other agent's pane.
2. **Look at the image before attaching it** (the `Read` tool shows it): no Vault values, tokens, keys, account ids, private repo names or paths, other agents' conversations, or the operator's personal data. Build from current main: since #5455 a `task dev` build keeps its own Kanban queue, so its Kanban shows the build's own tasks; a build from before that change still reads the operator's real task store and would show real task titles in a screenshot.
3. After a payment or ID entry in a browser pane, no screenshot (existing rule).

What the system does: `screenshot` already blacks out secret fields; the same redaction runs on every recording frame (§6); the operator can remove any file (§4.3). **Attachments stay on this machine:** the files live in the shared home, are served only through the authenticated local routes, and are never put in a task's PR text or sent to the cloud by this feature. Media for a PR on a public repo is a separate act under the public-repo rules (nothing internal visible), not something the Kanban does for the agent.

## 8. Cross-host

Agents on another host send media by `Message` files today, and they arrive on the recipient's machine as files with absolute paths (the `<attached_files>` block). Those paths are ordinary files on this machine, so **a task's `files` accepts them**: the recipient (the owner of the task here, or a leader) attaches what a remote agent sent, and `attached_by` is the agent that attached it. The caption says where it came from ("from Clare, macOS, via Message"), since the Kanban is per machine and a remote agent cannot call the local `Tasks` tool. The attachments store's sweep does not matter, because the file is copied on attach. No new cross-host route is added.

## 9. Phases

- **1. Attach and show (#4285).** `files` and `captions` on the three actions; the staging upload route; `task-media/` and `db_work_media`; `media_count` in `kanban.list`, the media list in `kanban.task`; the serving route and thumbnails; the card strip and the drawer section; `ui_change` and `no_media_reason` and the submit check; operator removal; the hourly prune; the `Tasks` tool text, the operator-config text about tasks (AgentY's lane: told before it changes), the seed note on the rule.
- **2. Recording (a follow-up task, after phase 1).** `UI record` as in §6, if the capture holds up in the CEF host; otherwise a report.

Files touched in phase 1 (all existing owners told first; the Kanban files are mine): `crates/srv/src/backend/storage/work_media.rs` (new), `server/work_media.rs` (new), `server/work_tasks.rs`, `server/app_api/kanban.rs`, `server/routes.rs`; `crates/mcp/src/tools/work.rs`, `tool_schemas.rs`; `frontend/app/view/warden-kanban/` (`kanban-media.tsx` new, card, drawer, types). The Kanban's view is moving into the Swarm (#2417); this adds new files beside it and touches the card and drawer, which move as they are.

## 10. Decisions for the operator

| # | Decision | Recommendation |
|---|---|---|
| **D1** | How a task is known to change the UI | A `ui_change` flag, set by whoever files or does the work, and toggled by the operator on the card |
| **D2** | How long media are kept | 30 days after the task finishes (as its history), unless the operator removes it sooner; one file at most 25 MiB (image) or 100 MiB (recording), 30 files and 300 MiB per task |
| **D3** | Missing media at submit | Refuse, with `no_media_reason` as the way through (shown on the card) |
| **D4** | Recordings | Phase 1 is images (a filmstrip meets the rule); phase 2 adds `UI record` if the capture proves out |
| **D5** | Who removes a file | Only the operator |

## 11. Tests

Each written to fail without its change.

- **srv:** an upload is hashed and typed by content (a `.png` that is not a PNG is refused; an SVG with a script is stored and served as an image only); the per-call, per-task and size limits; the same bytes stored once for two tasks; `submit` of a `ui_change` + `review_required` task without media is refused, passes with media from this attempt, passes with `no_media_reason`, and media from an earlier attempt does not count; a task not marked is unchanged; `update` with a PR link on a marked task without media carries the reminder; the hourly prune removes the media of a task 30 days after it finished and not before, and a blob still named by another task stays; `media_remove` refuses an agent caller and an unproven connection and keeps a `removed_at` row; a staging file older than an hour goes.
- **mcp:** `files` and `captions` on enqueue, update and submit; the path checks; a failed upload fails the call before the task changes; the schema lists the new parameters and the handler table matches.
- **frontend:** the card shows up to three thumbnails and "+N"; no strip for a task without media; the "no media" pill; the drawer lists newest attempt first with who/when/attempt; a click opens the Editor on the stored path; a recording does not autoplay.
- **Live (my own `task dev`, built from current main):** an image and a recording attached to a task filed in the build's own queue (#5455), the strip and the drawer in wide and narrow width.

## 12. Risks

- **The shared store location.** `task-media/` in the shared home is read by every channel and every build; an older build ignores it, a newer one reads it. The prune must be safe if two channels run it at once (it deletes a blob only when no row names it, checked in one transaction).
- **A flag agents can forget.** `ui_change` is only as good as whoever sets it; leaders filing UI work set it at `enqueue`, and the Tasks tool text says so. If it proves unreliable, the PR-files signal is the next step, once the poller has a better budget.
- **Recording may not be possible** in the engine's build on every OS (§6); phase 1 does not depend on it.
- **Size.** 300 MiB a task, many tasks: the 30-day removal and the per-task cap bound it, and a disk-space line in the Kanban footer could follow if it grows.

Agent: Naki-2@narko
