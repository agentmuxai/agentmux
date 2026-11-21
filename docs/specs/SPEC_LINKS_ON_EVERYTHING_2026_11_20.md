# Links on nearly everything: one recognition layer that turns agent names, task numbers, files, pull requests and URLs into correct links

**Status:** active — approved by the operator 2026-11-20 (decisions D1 to D9 as recommended, with the two security additions of §6.4 and §6.5); the resolver, the registry, the address, pull request, task and agent kinds, the indexes, the segment cache and the corpus gates (§4, §5, §9 build 1) ship in task #48249's first PR; the Kanban, the agent pane and the other surfaces (§7) are not started. Verified 2026-11-20.

## 1. What is asked

1. Nearly everything that names something a person could open should be a link, so they can move through the app quickly: above all on the Kanban, and in the agent pane.
2. The things named are: an agent (by name), a task (by number), a file (by name or path), a pull request, issue or commit, a web address, and a tab. A click goes where the thing is: an agent's pane, the Kanban at that card, the Editor at that file, a browser page, the tab.
3. The text is often written by an agent, so this must be a framework with a filter, not hand-placed links: it recognises what a piece of text names, checks it is real, and renders a link only when it is, on every surface that shows such text.
4. It is not a second way to open places. The click calls the app's deep-links framework (`SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md`, R11a: `openDestination` and its typed wrappers). This spec is the layer that finds the names in text and hands them to it.

A worked example (invented names): an agent's reply says "I asked Quill to send the #1234 audit table (findings.md) as an attachment". Three links appear: `Quill` opens Quill's pane, `#1234` opens the Kanban at that card, `findings.md` opens the Editor at that file. Each is a link only because the agent, the task and the file exist.

## 2. What exists today (read from the code)

| Piece | What it does now | Where |
|---|---|---|
| Markdown renderer | A remark and rehype chain; a custom link component; `rehypeLinkify` makes bare URLs links (the `linkify-it` layer of `SPEC_AGENT_PANE_HYPERLINKS_2026_06_20.md`); `rehypeSanitize` allows the default protocols plus `file` and `agentmux` | `frontend/app/element/markdown.tsx` (`buildProcessor`), `rehype-linkify.ts` |
| Who uses it | Agent assistant text; Memory and Global Memory | `view/agent/components/MarkdownBlockParts.tsx`, `view/memory-editor/MemoryContent.tsx`, `view/global-bundle/GlobalMemoryFullView.tsx` |
| Plain-text linkifier | `LinkifiedText` makes only URLs anchors, and only where `isSafeHref` (http, https, ftp, mailto, ssh, file) | `element/linkified-text.tsx`, `linkify-config.ts`; used by user messages, agent messages, the jekt body and the Messages panel |
| Click handling | `onLinkClick` and `onLinkAuxClick` choose between the Editor (files), the Kanban (`agentmux://task/N`), the system browser (web, through `openExternal`, which srv's host allows only for http, https and a few more) and a browser pane (middle click, Ctrl or Cmd click) | `element/link-open.ts`, `link-destinations.ts`, `link-target.ts` (`classifyHref`) |
| Task links | `agentmux://task/N` is parsed and opened; nothing recognises `#N` in text | `link-target.ts`, `util/open-kanban-task.ts` |
| Agent names | No recognition in text. The agent chips on the Kanban cards and drawer are plain `span`s; Swarm rows and Audit "pane" buttons reveal a pane by block id | `view/kanban/kanban-card.tsx`, `view/swarm/swarm-agent-row-hooks.ts`, `view/audit/audit-row.tsx` |
| File paths in text | No bare-path recognition; a path is a link only as a Markdown href or an image source. `isLikelyFilename` even drops `README.md`-style matches as web addresses | `linkify-config.ts` |
| Confinement of a container agent's files | The pane's media provider marks a container agent's pane `confined`; the click goes to the Editor with `message_of`; srv resolves it only inside that agent's own folders (`agent_media_files`, `container_folders::written_by_a_container_agent`, `agent_folder_open`) | `element/open-message-file.ts`, `crates/srv/src/.../agent_media_files.rs`, `agent_folder_open.rs` |
| Kanban text | Title, note, request quote, payload, checklist, result, history lines and the "asks" reason are plain text; no Markdown or linkifier in the view | `view/kanban/kanban-card.tsx`, `kanban-drawer.tsx`, `kanban-columns.ts` |
| Other text surfaces | Swarm status line, jekt and Messages header (plain), toasts (plain), Audit summary and reason (plain), Memory (Markdown) | `swarm-agent-card.tsx`, `MessagesPanel.tsx`, `notification/notificationitem.tsx`, `audit-row.tsx` |
| The roster | An agent definitions store, reactive, refreshed on `AgentsChanged`; `nameInAgents` is a linear find over slug and id only. The board has its own agent list per Kanban pane | `store/agent-names.ts`, `view/kanban/kanban-types.ts` |
| The board | One `KanbanModel` per pane holds every board's tasks and the selected board's view; a task by number is looked up preferring the default board (`taskForOpenRequestAnywhere`); a task reference is `#12`, or `work#12` on another board | `view/kanban/kanban-model.ts`, `kanban-boards.ts` |
| The framework to call | Not yet merged: the deep-links spec (R3, R11a) defines `openDestination` and the typed wrappers for an agent, a task by number, a file, a URL and a tab | `SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md` |

So the pieces for URLs and for a few opens exist, and nothing recognises a name, a number or a file in text. Each surface that wants a link today would have to write its own recogniser. That is what this spec prevents.

## 3. The entities, and what a click does

One row per kind of thing the recogniser knows. The kind is a file in a registry (§4), so a new kind is one file.

| Kind | Recognised as | Resolved against | A click opens | When it does not resolve |
|---|---|---|---|---|
| Agent | A roster name or slug as a whole word (`Quill`), a name with its computer (`Quill@charlie`), an agent id | The roster index (§4.3) | That agent's pane: reveal it if open, else open it (the deep-links `agent` destination) | Plain text |
| Task | `#N`, `task #N`, `task N`, `work#N` (a named board), `agentmux://task/N` | The task index (§4.3): the number on a board the viewer can see | The Kanban at that card (`kanban/task/N`) | Plain text |
| Pull request or issue | `PR 123`, `PR #123`, `pull 123`, `owner/repo#123`, a `github.com/.../pull/N` or `/issues/N` address | The repository the surface knows (§3.1), never one written in the text | A browser page (D1) | Plain text |
| File | A name with a known extension (`findings.md`), a relative path (`src/notes/plan.md`), an absolute path, with an optional `:line` | The author's workspace (§6) | The Editor at the file (and line) | A muted word with a reason on hover (§5) |
| Address | `http://` or `https://`, and the bare domains `linkify-it` finds today | Scheme allowlist: http and https only (§6) | A browser page, or the person's browser by the framework's rules | Plain text |
| Tab | "tab 2", "the other tab", only when the surface can say which window it is in | The window's tabs | That tab | Plain text. Later build (D8) |
| Commit | 7 to 40 hex characters next to the word `commit`, or `owner/repo@sha` | The surface's repository | A browser page | Plain text. Later build (D8) |

### 3.1 `#N`: a task or a pull request

The words around the number decide, in this order:

1. `PR`, `pull`, `pull request`, `issue` before it, or `owner/repo#N`, or a `/pull/N` address: a pull request or an issue.
2. `task` before it, `work#N`, `agentmux://task/N`: a task.
3. A bare `#N`: a task if the task exists on a board the viewer can see (the Kanban is this product's own number space); otherwise a pull request of the surface's repository if the surface has one (a task card knows its repository; an agent pane knows its working repository) and the number has three digits or more, so that "step #3" is never a pull request; otherwise plain text.

When a bare `#N` is both, the task wins, and the pull request stays reachable from the card (the card's own PR link). The link says which it is: its accessible name reads "Open task 1234" or "Open pull request 1234" (D2).

The repository of a pull request link always comes from the surface (the task's repository field, the pane's working repository), never from the text: text cannot point `PR 123` at a repository of its choosing, except by writing the full `owner/repo#123` or address, which is then an address (§6).

## 4. One framework

### 4.1 The resolver

A pure function, `linkify(text, context) -> Segment[]`, where a segment is plain text or a link: `{ kind, label, destination, state }`. `label` is the characters as written; `destination` is built from the entity, not from the text (§6); `state` is `ready`, `unchecked` or `dead` with a reason (§5).

The steps, the same on every surface:

1. **Protect.** Mark the ranges that are never touched: text already inside a link, an image, a code block and (by default) an inline code span (D4).
2. **Recognise.** Each registered kind scans the unprotected text with its own pattern and returns candidates (`kind`, range, the entity key it reads).
3. **Resolve.** Each candidate is looked up in an index (§4.3). No entity, no link.
4. **Choose.** Overlaps are settled by one rule: an address wins over a path, a path over a task, a task over a name; a longer match over a shorter; the earlier over the later.
5. **Emit** segments; the renderer draws them.

It is idempotent (running it on its own output's text changes nothing), keeps its input's text byte for byte (a link's label is the original characters, so copying the text copies the text), and is the same code for the Markdown pipeline and the plain-text surfaces.

### 4.2 Registering a kind

A kind is one file exporting `{ id, priority, recognise(text, ctx), resolve(candidate, ctx), verify?(entity, ctx), open(entity, ctx) }`. `open` calls the deep-links framework's typed wrapper (`openAgent`, `openTask`, `openFile`, `openUrl`, `openTab`) and nothing else: the recogniser never opens anything itself (deep-links R11a). A test fails when a kind opens a pane another way, and when a kind is not in the registry.

### 4.3 Indexes, cached

- **Agents:** a map from lower-cased display name, slug and id to the agent, built from the roster store and rebuilt when it changes (`AgentsChanged`). A first-letter set filters candidates before any lookup. Names that are ordinary words are handled by D3.
- **Tasks:** a map from number to `{ board, title, state }`. The Kanban pane already holds every board's tasks; the index is the same data held once for the window (one small store fed by the board refresh that already runs on `workqueue:changed`, not a per-surface fetch). A surface with no Kanban open still has it: the store loads the numbers only (a list call, not the task bodies).
- **Files:** no index; a candidate is `unchecked` until verified (§5).
- **Budget.** Linkifying a 10 KB message after the indexes exist takes no more than 2 ms; a transcript of 1,000 messages is never scanned whole: a message is linkified when it is first drawn and the segments are cached by (message id, text hash, index version). A change in the roster or the task index bumps the version, and drawn messages re-resolve lazily, only when they are visible.

### 4.4 Streaming

An agent's text arrives in pieces. A candidate that touches the end of the text so far is not linked until a delimiter follows it or the message is complete, so `Qui`, `#12` while `#1234` is being typed, and `findings.m` never flicker into a wrong link. The final text decides. A link that appeared stays a link only if the final text still holds it (a message is linkified again from its whole text when it completes).

### 4.5 One renderer

- **Markdown:** a rehype plugin, `rehypeEntities`, runs `linkify` over each text node that is not under a link, image or code, and replaces it with the segments; the existing custom `Link` component draws them and the existing click chain runs.
- **Plain text:** a `Linkified` component replaces `LinkifiedText`'s own scanning with the same function, so user messages, agent messages, the jekt body, the Messages panel, toasts, the Kanban text and Swarm lines share it.
- The URL recogniser is the existing `linkify-it` layer, moved behind the registry as the `address` kind; the existing behaviour for URLs is kept, with the scheme rule of §6.

## 5. Verification: a link only if it is real

| Kind | Check | When | A failure shows |
|---|---|---|---|
| Agent | The name is in the roster; if it names an agent on another computer, that computer's roster says so | At recognition (an index lookup) | No link |
| Task | The number is on a board the viewer can see | At recognition | No link |
| Pull request | The surface has a repository; the number is plausible (a number only; the existence check is the browser page's) | At recognition | No link when no repository |
| File | The path resolves, by srv, in the author's folders (§6), as a file | Lazily: on first hover or focus, once per path per author, cached for the session | A muted word, not a link, with the reason on hover: "not found", "outside this agent's folders" |
| Address | The scheme is http or https | At recognition | Plain text |

A file candidate is drawn as a link in a quiet state until it is checked, so a long transcript never makes a request per path: only the files the person hovers or tabs to are checked. A dead one then turns muted and says why. (Option: check the files of the visible messages in one batched call when a message completes; D6.)

**A test corpus.** Real agent messages and task cards, redacted, each with the links a person would expect, kept in the repo. The gates: **no false link** in the corpus (precision 100 percent on the labelled negatives: ordinary words that are agent names, `#` in a heading, a version `1.2.3`, a time `10:30`, an issue number of another project), and recall at 90 percent or better on the labelled positives. Every kind adds its own cases. A debug view (dev mode) lists what the recogniser saw in a surface: each candidate, its kind, what it matched and why it was or was not made a link.

## 6. Security

Text written by an agent is untrusted, and so is text written about one.

1. **A link's target comes from the entity, never from the text.** `Quill` can open Quill's pane and nothing else; `#1234` opens the Kanban at task 1234; a path opens the Editor, not a command. The text only chooses which entity, and only from the indexes. There is no way to write a target.
2. **Markdown links with a deceiving label.** An agent can write `[Quill](https://example.test)`. A Markdown link keeps its own href, shown on hover; when its label is itself a recognised entity (an agent name, a task, an address) and the href is not that entity's own, the link is drawn with a warning mark and its real address in the accessible name and the hover. (This is the deep-links G8 rule for "a link whose text is a different link", applied to labels.)
3. **Schemes.** A link made from recognised text is http or https, or a destination of the app; never `javascript:`, `file:`, `data:`, `ssh:` or `ftp:`. The existing `isSafeHref` list (which allows file, ssh, ftp and mailto) is tightened for text-derived links; a path is a `file` kind resolved by srv, not a `file:` link.
4. **Files and the container agent.** A file name resolves only in the author's own folders. For a host agent that is its working directory and the folders its task names; for a container agent it is its own folders only, by the existing path (the pane is `confined`, the click carries `message_of`, srv resolves with `agent_media_files` and stamps who wrote it). The recogniser never resolves a path on the client: it asks srv to check and to open, with the author in the request. A path outside the author's folders is a muted word, not a link, and the reason does not name the host path.
   srv's check is built to be safe on text full of hostile paths: it **canonicalises before it compares** (symlinks, `..`, junctions, UNC paths and 8.3 short names are all resolved first, and the result, not the written path, is compared with the author's folders); it is **read-only** (it checks that a file exists and reads nothing); and it is **rate-limited per window and per author**, so text full of paths cannot be used to probe the disk. An srv test covers a symlink escape and a `..` escape, each of which must come back "outside this agent's folders".
5. **Authors.** Every surface passes `context.author`: who wrote the text (a host agent, a container agent, the operator, the app) and on which computer. A task card's author for its payload is its filer; for a result its holder; for a history line its actor. A surface with no author (a toast the app wrote) resolves files against nothing and so makes no file links. **A task from another computer** (a remote board) gets no file links at all, only agent, task and pull request links: its file words are plain or muted, with no path in the hover, because this computer cannot vouch for that computer's folders.
6. **Opening never acts.** A click opens a place and never runs, sends, starts a turn or changes anything. An address opens in the person's browser only by their click and never by itself. No link auto-opens, in any state.
7. **Names from other computers** link to a remote pane if one exists, else a read-only card of that agent (the deep-links `agent` rules); a link never carries the other computer's paths.
8. **Bidirectional and control characters** in a candidate are stripped before the match, as in the deep-links G8; a candidate that was changed by it is not linked.

## 7. The surfaces, in order

| Order | Surface | Text it holds | Cost |
|---|---|---|---|
| 1 | **Kanban**: card title and note; the drawer's request quote, payload, checklist, result, history lines, the "asks" reason; the assignee and holder chips | Task text, written by agents and the operator | Replace the plain-text renders with `Linkified`; make `AgentChip` an entity link (it is not text, so no recognition: it is the agent link of §3 with the agent known). Small |
| 2 | **Agent pane**: assistant text (Markdown), user messages, tool-row descriptions, reasoning | Agent-written | `rehypeEntities` in the Markdown chain; `Linkified` in the plain components. Small to medium (streaming, §4.4) |
| 3 | Swarm status lines and the jekt and Messages panel (header names and body) | Agent-written, short | `Linkified`; the sender names as agent links. Small |
| 4 | Toasts and notices | The app's words and agent text | `Linkified` (the deep-links spec makes the app's own place references links; this adds names in them). Small |
| 5 | Audit rows | Summaries and reasons | `Linkified`. Small |
| 6 | Memory entries (Markdown) | Written by agents and the operator | `rehypeEntities`; the author is the memory's scope. Small |
| 7 | MuxAPI-written status lines, docs previews, Settings help | Short | Last; only where agent-written text appears |

The app's own toasts and notices, which have no author, make no file links (§6.5); they link names, tasks and the places the app itself names.

**Not linkified, by default:** fenced code blocks; text inside an existing link or image; terminal output (a different framework: the shell's own link handling); an inline code span, with one exception (D4): a code span whose entire content is a single path that verifies, or a single task number, is linkified, because that is how agents most often write a file name.

## 8. How a link looks and behaves

- **Look.** Entity links (agent, task, file, pull request) use the app's link colour with a dotted underline, so they read as part of the sentence; an address keeps the solid underline it has now with a small external mark. A muted word (a dead file) has no underline and the muted text colour. The toasts' underlined links are the same family (D5).
- **Zoom.** A link's size is its text's: in the agent pane and the Kanban it follows pane zoom; in a toast, chrome zoom.
- **Keyboard.** Each link is an anchor: Tab reaches it, Enter opens it; the order is reading order. Ctrl or Cmd with a click, and a middle click, keep their meaning (a floating or new pane).
- **Screen reader.** The accessible name says what a click does, not the characters: "Open agent Quill", "Open task 1234", "Open file findings.md in the Editor", "Open pull request 1234". A dead word reads as its text with the reason as its description.
- **Hover.** The target's name and kind in a tooltip now; a preview card is a later option and not in this spec.

## 9. Build order, in small PRs

1. **The resolver, the registry and the tests**: `linkify`, the kinds for address (moved), task and agent, the indexes, the streaming rule, the corpus and its gates. No surface changes.
2. **The Kanban** (the first ask): `Linkified` for the card and drawer text, `AgentChip` as a link, the task index store. Look checks on macOS and Linux.
3. **The agent pane**: `rehypeEntities`, the plain components, files with srv's verify call and the confinement. Look checks.
4. **The other surfaces**: Swarm, Messages, toasts, Audit, Memory.
5. Later: tab and commit kinds, the preview card.

Each is a PR with a changeset. Build 1 needs the deep-links framework's typed wrappers (`openAgent`, `openTask`, `openFile`, `openUrl`); until they are merged, the recogniser's `open` is a thin adapter over the existing `openKanbanTask`, `openPathLink` and `openLink`, replaced by the wrappers when they land.

## 10. Tests

| What | How |
|---|---|
| The recogniser, per kind | Table tests: candidates and non-candidates, boundaries (punctuation, brackets, Markdown emphasis), Unicode names |
| `#N` | The ordering of §3.1, one case each, including both a task and a pull request |
| Protection | A candidate inside a link, an image, a code block, an inline code span is left alone; the code-span exception of D4 |
| Streaming | A text fed in pieces never links a prefix; the final text links once |
| Idempotence and fidelity | The segments' labels joined equal the input; linkifying twice changes nothing |
| Security | A Markdown label that looks like another entity is flagged; `javascript:` and `file:` are never made; a path outside the author's folders is muted; a container author cannot resolve a host path (srv test) |
| Budget | A 10 KB message and a 1,000-message transcript within the budget of §4.3 |
| The corpus | The precision and recall gates of §5 |
| Surfaces | Each surface renders a known text with the expected links (component tests), and keeps its text when the roster is empty |

## 11. Decisions wanted

| # | Question | Recommendation |
|---|---|---|
| D1 | A pull request or address opens in an in-app Browser pane, or the person's own browser? | The person's browser by default (what a web link does now); middle click and Ctrl or Cmd click open the Browser pane. A setting later if wanted |
| D2 | A bare `#N` that is both a task and a pull request? | The task, with the pull request reachable from the card; the accessible name says which |
| D3 | An agent whose name is an ordinary word (a first name or a common word)? | Link a name only when written with its capital as in the roster, as a whole word; add a short "never alone" list of names that are also ordinary words, which, in any case, link only in the `Name@computer` form or by an id that is more than the word |
| D4 | Inline code spans? | Not linkified, except a span that is wholly one verified path or one task number |
| D5 | The look: dotted underline for entity links, solid for web addresses? | Yes; an icon per kind is not added in the first build |
| D6 | When a file is checked: on hover only, or a batched check of the visible messages when a message completes? | Hover and focus first; the batch if the quiet-then-muted change is too slow to notice in use |
| D7 | Container agents' file names? | Resolve only in their own folders through srv, as the confined click does today; never on the client |
| D8 | Tab references and commit hashes? | Not in the first builds; add as kinds later |
| D9 | The scheme list for text-derived links? | http and https only; mailto is a separate decision if wanted |

## 12. Relation to other work

- **The deep-links framework** (`SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md`): this spec depends on R11a. What this spec asks of it: typed wrappers for an agent (by id), a task (by number, with an optional board), a file (path, line, and the author for confinement), an address and a tab; the `agent` destination, which that spec marks as its second build, ahead of the agent links here; and a cheap "can this open" answer for a destination, so a link can be drawn muted without opening anything.
- **`SPEC_AGENT_PANE_HYPERLINKS_2026_06_20.md`**: the URL layer. Kept; moved behind the registry as the `address` kind.
- **The command palette and the status bar** are unaffected.

## 13. Left alone

- The terminal's own link handling.
- How a Browser pane or the Editor opens things once asked.
- A hover preview card, and linking inside images.
- The existing Markdown links an agent writes by hand (a real href is kept as it is; only a deceiving label is flagged).
