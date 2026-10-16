# SPEC: MuxAPI in fourteen tools: one per family, with actions

**Status:** active — all slices built: S1 (`Terminal`, #5001), S2 (`Browser`, `UI`, #5003), S3 (`Memory`, `Tasks`, `Presence`, `Schedule`, #5006), S4a (`Pane`, `Stop`, `Widget`, `Identity`, #5008) and S4b (`Agents`, `Message`); remaining: the eval that measures the consolidated surface (§7).
**Author:** Agent5@narko, 2026-10-15, at the operator's request
**Trigger (operator, 2026-10-15):** "can we use fewer more powerful tools? like one Terminal tool that covers all those features?" … "lets take this opprunutiy to catalog all of agentmux's tools, we may want to do a similar simplification across the board, write a spec to file"
**Builds on:** `SPEC_AGENT_TERMINAL_DRIVING_2026_10_15.md` (the first family done this way: one `Terminal` tool with an `action`), `SPEC_AGENT_TASK_EVAL_2026_10_13.md` (the eval that measures what MuxAPI costs an agent).

## 1. Summary

1. **Today:** `agentmux-mcp` lists **95 tools** (0.59.26; `main` adds `PostToChannel` and `ReadChannel`, so 97), **88,158 characters** of tool definitions, 46,960 of them descriptions (§2, full catalog in Appendix A). They fall into 14 families; Browser alone is 17 tools and 15 KB.
2. **The cost is real even where tools are deferred.** Claude Code defers MCP tools behind ToolSearch, so an agent pays 97 names on every turn and a lookup per tool on first use; providers that don't defer pay all 88 KB up front. Published measurements put tool-selection accuracy in decline past roughly 30–50 visible tools, earlier for smaller models (§3).
3. **The proposal: 14 tools, one per family**, each with an `action` parameter and a flat set of optional parameters (§4). Destructive operations stay in their own tool so a harness can gate them. Tools an agent can't use (container-only, host-only) aren't listed to it.
4. **Common sequences become one call** (type then wait then read; click then wait then snapshot), because round trips cost more than tool definitions.
5. **Migration** in four slices, one family group at a time, with every reference (the Operator Config "MuxAPI" entry every agent reads, tips, the agent pane's tool rows, tests, docs) updated in the same PR (§6). Each slice is measured with the eval before the next one (§7).

## 2. The catalog (installed 0.59.26, `tools/list`, 2026-10-15)

Measured by running the installed `agentmux-mcp` and asking it for `tools/list` (it answers from a static list). "Chars" is the JSON of each tool definition as sent: name, description and input schema.

| Family | Tools | Chars | Members |
|---|---|---|---|
| Browser | 17 | 15,082 | OpenBrowser, BrowserSnapshot, BrowserClick, BrowserFill, BrowserSelect, BrowserCheck, BrowserSetFiles, BrowserWaitFor, BrowserHandoff, BrowserNavigate, BrowserBack, BrowserForward, BrowserReload, BrowserEval, BrowserDispatchKey, BrowserFocusElement, BrowserFocusInfo |
| UI automation | 8 | 10,296 | UIScreenshot, UIClick, UIQuery, ListShortcuts, RunCommand, PressKeys, CaptureWindow, DiscoverWindows |
| Memory | 13 | 9,123 | MemoryList, MemoryRead, MemoryWrite, MemoryHistory, MemoryDiff, MemoryRevert, GlobalMemoryList, GlobalMemoryRead, GlobalMemoryWrite, GlobalMemoryRemove, GlobalMemoryHistory, GlobalMemoryDiff, GlobalMemoryRevert |
| Terminal | 11 | 8,714 | Shell, ShellInput, ShellStatus, ShellStop, PtyShell, PtyShellInput, PtyShellResize, PtyShellRead, PtyShellStatus, PtyShellStop, ConnList |
| Agents | 7 | 8,702 | DiscoverAgents, FleetList, OpenAgent, GetAgentTranscript, ListConversations, SearchHistory, SupervisorNudge |
| Panes and windows | 10 | 8,012 | WhoAmI, OpenEditor, OpenMedia, OpenFiles, NewTab, SetActiveTab, FocusWindow, Layout, SetName, ClosePane |
| Task queue | 6 | 6,555 | WorkEnqueue, WorkClaim, WorkHeartbeat, WorkComplete, WorkRelease, WorkList |
| Schedule | 8 | 5,884 | CronCreate, CronList, CronPause, CronResume, CronDelete, Loop, LoopList, LoopStop |
| Presence | 3 | 4,401 | WhoIsWorkingOn, ClaimWork, ReleaseWork |
| Stop | 2 | 3,628 | QuitSelf, FleetBulkStop |
| Messaging | 2 (+2 on main) | 2,896 | SendMessage, FleetBroadcast; PostToChannel, ReadChannel on main |
| Widgets | 3 | 2,238 | WidgetList, WidgetInstall, OpenWidget |
| Presets and identities | 4 | 1,577 | PresetList, PresetGet, IdentityAccounts, IdentityValidate |
| Dev server | 1 | 1,050 | RegisterDevServer (container agents only) |
| **Total** | **95** | **88,158** | |

The largest single definitions: SearchHistory 3,267, CaptureWindow 2,523, WhoIsWorkingOn 2,512, OpenBrowser 2,436, WorkEnqueue 2,205, CronCreate 2,128, PtyShell 2,038.

Things the catalog shows besides size:

- **Confusable names.** `WorkClaim` (take a task from the queue) and `ClaimWork` (say what you're working on) are different tools in different families. `Loop` and `CronCreate` both schedule repeated work. `FleetList` and `DiscoverAgents` both list agents. `UIScreenshot` and `CaptureWindow` both capture pixels.
- **Repeated boilerplate.** The 17 Browser descriptions each restate the pane-ownership rule, the untrusted-content warning and the `pane` parameter; the 13 Memory tools restate Personal versus Global; the Cron tools restate the schedule format.
- **Tools listed to agents that can't use them.** `RegisterDevServer` is for container agents only, and the widget tools for host agents only, but every agent is shown all of them.

## 3. What the guidance says

- **Consolidate related operations behind an `action` parameter.** Anthropic's tool docs: "Rather than creating a separate tool for every action (create_pr, review_pr, merge_pr), group them into a single tool with an action parameter. Fewer, more capable tools reduce selection ambiguity" ([Define tools](https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools)). Anthropic's own text-editor and computer-use tools are shaped this way.
- **Build for workflows, not endpoints, and namespace clearly** ([Writing effective tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents)): fewer, higher-leverage tools cut tool calls and context and lower the chance of errors between calls; overlapping or vague tools confuse agents; the naming scheme matters measurably, so choose it by evaluation.
- **GitHub's MCP server did exactly this.** It merged its pull-request and issue tools into fewer tools with a method parameter, replaced six Projects tools with three and cut about 23K tokens (50%), trimmed its default toolsets, and shows tools by what the user is authenticated for ([changelog, Oct 2025](https://github.blog/changelog/2025-10-29-github-mcp-server-now-comes-with-server-instructions-better-tools-and-more/), [changelog, Jan 2026](https://github.blog/changelog/2026-01-28-github-mcp-server-new-projects-tools-oauth-scope-filtering-and-new-features/)); it also offers dynamic toolsets that start with a handful of tools ([server configuration](https://github.com/github/github-mcp-server/blob/main/docs/server-configuration.md)).
- **Tool count hurts selection.** Reported thresholds vary, but they cluster at 30–50 visible tools, with smaller models degrading from 10–15 ([MCP tool overload](https://getunblocked.com/blog/mcp-tool-overload/), [MCP tool limits](https://fast.io/resources/mcp-tool-limit/), [LiveMCPBench](https://arxiv.org/pdf/2508.01780)). 97 is past every one of them.
- **The counter-argument:** a read-only lookup and a destructive operation are worth keeping apart so they can carry different safety controls (third-party guidance, consistent with how harness permissions work: Claude Code's allow and ask rules match tool names).

## 4. The proposal

### 4.1 Rules for every family tool

1. **One tool per family, named by the noun:** `Terminal`, `Browser`, `Memory`, … (MCP clients already prefix them, e.g. `mcp__agentmux__Browser`).
2. **`action` is a required enum; every other parameter is optional and flat.** Each parameter's description names the actions it applies to. No `oneOf`/`anyOf` per action: some providers' function-calling schemas reject or flatten them. srv validates the combination and returns an error that names what applies.
3. **Shared rules are written once,** in the tool's description, not per action: ownership, untrusted content, scopes, formats.
4. **Compose.** An action that is usually followed by a wait and a read takes them as parameters (`Terminal` `type` with `wait` and `read`; `Browser` `click` with `wait_for` and `snapshot`). Each saved round trip is a model turn.
5. **One result shape:** `{ ok, …data, error?: { code, message, hint } }`, with cursors and `truncated` flags where output can be long, and token-saving defaults (tails, diffs, limits).
6. **Destructive operations live in their own tool** (`Stop`), so a harness can ask before them while allowing the rest. srv's own guards apply regardless of what the harness allows.
7. **Only what this agent can use is listed:** container-only and host-only tools are left out of `tools/list` for agents that can't use them (srv knows the agent's kind at spawn).

### 4.2 The fourteen tools

| Tool | Replaces | Actions |
|---|---|---|
| `Terminal` | Shell×4, PtyShell×6, ConnList | `open`, `type`, `read`, `wait`, `run`, `close`, `connections` (per `SPEC_AGENT_TERMINAL_DRIVING_2026_10_15.md`; the pipe `Shell` becomes `run` with `background: true`) |
| `Browser` | OpenBrowser, Browser×16 | `open`, `navigate`, `back`, `forward`, `reload`, `snapshot`, `click`, `fill`, `select`, `check`, `set_files`, `key`, `focus`, `focus_info`, `wait_for`, `eval`, `handoff`; `click`/`fill`/`navigate` take `wait_for` and `snapshot` |
| `Memory` | Memory×6, GlobalMemory×7 | `list`, `read`, `write`, `remove`, `history`, `diff`, `revert`, with `scope: "personal" | "global"` (default personal) |
| `Tasks` | Work×6 | `enqueue`, `claim`, `heartbeat`, `complete`, `release`, `list` (renamed from "Work" so it can't be confused with Presence) |
| `Presence` | WhoIsWorkingOn, ClaimWork, ReleaseWork | `who` (by path, branch, topic or PR), `claim`, `release` |
| `Schedule` | Cron×5, Loop×3 | `create` (a cron expression, an interval, or a self-paced loop), `list`, `pause`, `resume`, `delete` |
| `Agents` | DiscoverAgents, FleetList, OpenAgent, GetAgentTranscript, ListConversations, SearchHistory, SupervisorNudge | `list` (local, LAN, cloud; with fleet status), `open`, `transcript`, `conversations`, `search_history`, `nudge` |
| `Message` | SendMessage, FleetBroadcast, PostToChannel, ReadChannel | `send` (one agent), `broadcast` (several), `post` and `read` (channels) |
| `Stop` | QuitSelf, FleetBulkStop, ClosePane | `self`, `agents`, `pane`; kept separate because all three end work (rule 6). `pane` takes a `block_id`: ClosePane's no-argument form, which ended the caller's own session without QuitSelf's checks, is gone, and ending your own session is `self` only |
| `Pane` | WhoAmI, OpenEditor, OpenMedia, OpenFiles, NewTab, SetActiveTab, FocusWindow, Layout, SetName | `whoami`, `open` (`view: "editor" | "media" | "files"`), `new_tab`, `activate_tab`, `focus_window`, `layout`, `rename` (closing a pane is in `Stop`) |
| `UI` | UIScreenshot, UIClick, UIQuery, ListShortcuts, RunCommand, PressKeys, CaptureWindow, DiscoverWindows | `screenshot` (a pane or, with `window`, a whole window), `query`, `click`, `keys`, `shortcuts`, `command`, `windows` |
| `Widget` | WidgetList, WidgetInstall, OpenWidget | `list`, `install`, `open` (host agents only) |
| `Identity` | PresetList, PresetGet, IdentityAccounts, IdentityValidate | `presets`, `preset`, `accounts`, `validate` |
| `DevServer` | RegisterDevServer | `register` (container agents only). Not built: a tool with one action only adds a parameter, so RegisterDevServer stays as it is |

97 tools become 14; a host agent sees 13 (no `DevServer`), a container agent 13 (no `Widget`).

### 4.3 Expected size

Not estimated by hand: each slice's PR measures its family's `tools/list` JSON before and after and states both numbers. The target is that the 14 definitions together come to **at most half** of today's 88 KB, with no description losing a rule an agent needs (an ownership rule, a safety rule, a format). GitHub's Projects consolidation, the closest comparison, halved its tokens.

### 4.4 What doesn't change

- srv's HTTP endpoints. Each action calls the endpoint its old tool called; `agentmux-mcp` maps actions to endpoints. Scripts and `muxsh` that call srv directly are unaffected.
- Behaviour and permissions inside srv (ownership, locks, approvals, the browser's site limits).

## 5. Exposure and loading

- **Deferred clients** (Claude Code): 14 names instead of 97 on every turn, and one ToolSearch per family instead of per operation.
- **Clients that load every schema:** the whole surface is the §4.3 budget. If the eval shows it still costs too much for them, the next step is server-side toolsets: a core set always listed (`Pane`, `Memory`, `Message`, `Presence`, `Terminal`), the rest listed on request through `tools/list_changed`, for clients that honour it. Not part of this spec until the numbers ask for it.
- **Per-agent listing** (rule 7) is done at spawn from the agent's kind; it needs no client support.

## 6. Migration

Model calls can only use tools that are listed, so an old name kept as an alias but not listed helps no agent; a listed alias costs what it was meant to save. Each slice therefore **replaces** its family's tools in one release, and updates every reference in the same PR:

| Reference | Where | What changes |
|---|---|---|
| The tool list every agent reads | `crates/srv/operator-config-seed.json`, the "MuxAPI" entry | the families and their actions |
| Tool schemas and handlers | `crates/mcp/src/tool_schemas.rs`, `crates/mcp/src/tools/*` | one schema per family, an action router per family |
| The agent pane's tool rows | `frontend/app/view/agent/tool-meta/*`, `components/tool-header*` | rows are labelled by family and action (`Browser · click`), not tool name |
| Tips and help | `frontend/app/keybindings/tips.ts`, help pane | names |
| Other runtimes | `blockcontroller/acp.rs`, `app_server_controller.rs` (tool names for non-Claude providers) | names |
| Tests and docs | about 30 test files and 60 docs mention MuxAPI tool names (`SendMessage` alone: 27 code files, 59 docs) | names, in the same PR |

Agents' own memories and skills that name old tools are outside the repo; the release notes list the old → new mapping, and an unknown-tool call to `agentmux-mcp` with an old name returns an error naming its replacement.

## 7. Slices and measurement

| Slice | Families | Why this order |
|---|---|---|
| **S1** | `Terminal` | already specified; fixes a real failure |
| **S2** | `Browser`, `UI` | the two largest (25 KB together) and the most sequential use, so composition saves the most |
| **S3** | `Memory`, `Tasks`, `Presence`, `Schedule` | CRUD families; the confusable names go |
| **S4** | `Pane`, `Stop`, `Widget`, `Identity` (S4a), then `Agents`, `Message` (S4b) | the rest, in two PRs so each review stays small; `SendMessage` has the most references, so it goes last, when the pattern is settled |

**Order with the eval (operator, 2026-10-15: "get this in before we even try evals").** All four slices land first, each measured by its `tools/list` size before and after and by its own tests; the eval (`SPEC_AGENT_TASK_EVAL_2026_10_13.md`) then measures the consolidated surface against the pre-consolidation one (0.59.26's 95 tools, kept as an eval arm): the coding tasks for overhead (tokens, turns, pass rate), plus a small task set per family (driving a terminal, filling a form in a browser pane, writing and reading memory) for selection accuracy: the right tool and action chosen, errors from bad parameter combinations, calls per task. A family that comes out worse is reworked.

**Built so far:**

| Slice | PR | Before → after (`tools/list` characters) |
|---|---|---|
| S1 `Terminal` | #5001 | 11 tools, 8,714 → 1 tool, 4,602 (−47%); 97 tools → 87 |
| S2 `Browser`, `UI` | #5003 | Browser 17 tools, 15,082 → 1, 5,935 (−61%); UI 8 tools, 10,296 → 1, 3,185 (−69%); 87 tools → 64. The Browser actions that change a page take `until_text`/`until_url`/`until_gone` and `snapshot: true`, so act, wait and read is one call. Operator Config gains a Browser line (it had none) |
| S3 `Memory`, `Tasks`, `Presence`, `Schedule` | #5006 | Memory 13 tools, 9,123 → 1, 2,442 (−73%); Tasks 6, 6,555 → 1, 2,969 (−55%); Presence 3, 4,401 → 1, 2,643 (−40%); Schedule 8, 5,884 → 1, 2,167 (−63%); 64 tools → 38. Memory takes a `scope` (personal or global); Schedule's `create` takes `cron` (a persistent job) or `every` (a loop for the session). The channel tools pass through Presence until S4 |
| S4a `Pane`, `Stop`, `Widget`, `Identity` | #5008 | Pane 9 tools, 6,771 → 1, 3,447 (−49%); Stop 3, 4,915 → 1, 3,850 (−22%); Widget 3, 2,238 → 1, 2,175 (−3%); Identity 4, 1,577 → 1, 1,099 (−30%); 38 tools → 23, and the whole list 55,962 → 51,071 characters. `Pane` `open` takes a `view`; `Stop` `pane` needs a `block_id`, and `self` refuses any target. The agent tools left for S4b pass through unchanged |
| S4b `Agents`, `Message` | this PR | Agents 7 tools, 8,715 → 1, 6,464 (−26%); Message 4, 6,699 → 1, 5,584 (−17%); 23 tools → 14. `Agents` `list` is DiscoverAgents and FleetList, which were the same call; `nudge` takes `decision` (the old tool's own `action`); `search_history` refuses an `agent`, as SearchHistory had none. The Presence pass-through for the channel tools is gone |

**All slices together:** 95 tools, 88,158 characters (0.59.26) → 14 tools, 47,709 (−46%). That is short of §4.3's target of half (44 KB): most of what is left is the Terminal, Browser, Agents and Message descriptions, which carry the delivery answers, ownership rules and safety rules an agent needs. Trimming them further is for the eval to justify, not done blind.

S1 also moved the advertised list into one `ADVERTISED` constant (`crates/mcp/src/tool_schemas.rs`), which the server sends and a test checks, so each later slice changes the list in one place.

## 8. Open questions

1. **`Agents` and `Message` as one tool?** Kept apart (S4b): messaging is the most-used operation and reads clearest on its own, and the two descriptions together (12 KB) would make one tool larger than any other.
2. **`Pane` and `UI` as one?** `Pane` acts on layout through MuxAPI; `UI` drives the rendered app. They are kept apart because `UI` is the riskier surface (it can click anything), but they overlap at screenshots.
3. **Action names.** Verbs (`open`, `close`) versus noun-verb (`tab_new`). Anthropic notes the naming scheme affects selection; settle it with the S2 measurement and apply it to later slices.
4. **Structured outputs.** MCP's newer output schemas let clients that support them parse results without reading prose. Add them per family as slices land, where the client supports the spec version.

## Appendix A. Every tool (0.59.26)

Description characters / total definition characters / parameters (required).

| Tool | Proposed | Desc | Total | Params (req) |
|---|---|---|---|---|
| DiscoverAgents | Agents | 671 | 761 | 0 (0) |
| FleetList | Agents | 277 | 362 | 0 (0) |
| GetAgentTranscript | Agents | 692 | 1,034 | 2 (1) |
| ListConversations | Agents | 670 | 763 | 0 (0) |
| OpenAgent | Agents | 751 | 1,169 | 3 (1) |
| SearchHistory | Agents | 1,630 | 3,267 | 8 (1) |
| SupervisorNudge | Agents | 861 | 1,346 | 3 (2) |
| BrowserBack | Browser | 325 | 596 | 1 (0) |
| BrowserCheck | Browser | 153 | 583 | 3 (2) |
| BrowserClick | Browser | 345 | 688 | 2 (1) |
| BrowserDispatchKey | Browser | 660 | 1,299 | 4 (0) |
| BrowserEval | Browser | 590 | 1,098 | 3 (1) |
| BrowserFill | Browser | 265 | 694 | 3 (2) |
| BrowserFocusElement | Browser | 308 | 717 | 2 (1) |
| BrowserFocusInfo | Browser | 336 | 610 | 1 (0) |
| BrowserForward | Browser | 256 | 528 | 1 (0) |
| BrowserHandoff | Browser | 482 | 924 | 3 (1) |
| BrowserNavigate | Browser | 272 | 625 | 2 (1) |
| BrowserReload | Browser | 196 | 570 | 2 (0) |
| BrowserSelect | Browser | 217 | 643 | 3 (2) |
| BrowserSetFiles | Browser | 558 | 996 | 3 (2) |
| BrowserSnapshot | Browser | 814 | 1,224 | 2 (0) |
| BrowserWaitFor | Browser | 243 | 851 | 5 (0) |
| OpenBrowser | Browser | 1,257 | 2,436 | 5 (1) |
| RegisterDevServer | DevServer | 632 | 1,050 | 2 (2) |
| IdentityAccounts | Identity | 296 | 388 | 0 (0) |
| IdentityValidate | Identity | 241 | 457 | 1 (1) |
| PresetGet | Identity | 185 | 411 | 2 (0) |
| PresetList | Identity | 235 | 321 | 0 (0) |
| GlobalMemoryDiff | Memory | 204 | 661 | 3 (3) |
| GlobalMemoryHistory | Memory | 475 | 697 | 1 (1) |
| GlobalMemoryList | Memory | 581 | 673 | 0 (0) |
| GlobalMemoryRead | Memory | 92 | 277 | 1 (1) |
| GlobalMemoryRemove | Memory | 282 | 481 | 1 (1) |
| GlobalMemoryRevert | Memory | 621 | 947 | 2 (2) |
| GlobalMemoryWrite | Memory | 830 | 1,946 | 4 (2) |
| MemoryDiff | Memory | 106 | 426 | 2 (2) |
| MemoryHistory | Memory | 325 | 536 | 1 (1) |
| MemoryList | Memory | 264 | 350 | 0 (0) |
| MemoryRead | Memory | 128 | 324 | 1 (1) |
| MemoryRevert | Memory | 321 | 626 | 2 (2) |
| MemoryWrite | Memory | 259 | 1,179 | 3 (2) |
| FleetBroadcast | Message | 650 | 1,029 | 2 (2) |
| SendMessage | Message | 1,539 | 1,867 | 2 (2) |
| ClosePane | Pane | 961 | 1,287 | 2 (0) |
| FocusWindow | Pane | 169 | 352 | 1 (0) |
| Layout | Pane | 497 | 709 | 1 (0) |
| NewTab | Pane | 108 | 260 | 1 (0) |
| OpenEditor | Pane | 214 | 1,464 | 6 (1) |
| OpenFiles | Pane | 348 | 1,595 | 6 (1) |
| OpenMedia | Pane | 241 | 860 | 4 (1) |
| SetActiveTab | Pane | 141 | 320 | 1 (1) |
| SetName | Pane | 393 | 876 | 3 (2) |
| WhoAmI | Pane | 207 | 289 | 0 (0) |
| ClaimWork | Presence | 718 | 1,477 | 6 (0) |
| ReleaseWork | Presence | 220 | 412 | 1 (0) |
| WhoIsWorkingOn | Presence | 1,788 | 2,512 | 5 (0) |
| CronCreate | Schedule | 735 | 2,128 | 6 (4) |
| CronDelete | Schedule | 189 | 377 | 1 (1) |
| CronList | Schedule | 224 | 308 | 0 (0) |
| CronPause | Schedule | 247 | 408 | 1 (1) |
| CronResume | Schedule | 131 | 294 | 1 (1) |
| Loop | Schedule | 916 | 1,758 | 5 (1) |
| LoopList | Schedule | 176 | 260 | 0 (0) |
| LoopStop | Schedule | 157 | 351 | 1 (1) |
| FleetBulkStop | Stop | 969 | 1,681 | 3 (1) |
| QuitSelf | Stop | 1,393 | 1,947 | 2 (2) |
| WorkClaim | Tasks | 809 | 1,281 | 2 (0) |
| WorkComplete | Tasks | 331 | 893 | 3 (3) |
| WorkEnqueue | Tasks | 563 | 2,205 | 8 (2) |
| WorkHeartbeat | Tasks | 353 | 737 | 3 (2) |
| WorkList | Tasks | 246 | 533 | 2 (0) |
| WorkRelease | Tasks | 488 | 906 | 3 (2) |
| ConnList | Terminal | 398 | 482 | 0 (0) |
| PtyShell | Terminal | 983 | 2,038 | 4 (0) |
| PtyShellInput | Terminal | 362 | 676 | 2 (2) |
| PtyShellRead | Terminal | 303 | 605 | 2 (1) |
| PtyShellResize | Terminal | 136 | 472 | 3 (3) |
| PtyShellStatus | Terminal | 81 | 288 | 1 (1) |
| PtyShellStop | Terminal | 401 | 606 | 1 (1) |
| Shell | Terminal | 336 | 1,804 | 6 (1) |
| ShellInput | Terminal | 543 | 847 | 2 (2) |
| ShellStatus | Terminal | 230 | 431 | 1 (1) |
| ShellStop | Terminal | 266 | 465 | 1 (1) |
| CaptureWindow | UI | 1,521 | 2,523 | 3 (0) |
| DiscoverWindows | UI | 1,161 | 1,825 | 2 (0) |
| ListShortcuts | UI | 427 | 520 | 0 (0) |
| PressKeys | UI | 1,145 | 1,538 | 2 (1) |
| RunCommand | UI | 966 | 1,340 | 2 (1) |
| UIClick | UI | 453 | 850 | 2 (1) |
| UIQuery | UI | 523 | 993 | 3 (1) |
| UIScreenshot | UI | 437 | 707 | 1 (0) |
| OpenWidget | Widget | 406 | 832 | 3 (1) |
| WidgetInstall | Widget | 546 | 1,011 | 3 (1) |
| WidgetList | Widget | 309 | 395 | 0 (0) |

`main` also has `PostToChannel` and `ReadChannel` (Message), not in 0.59.26.
