# REPORT: Computer use for agents: where AgentMux stands, the state of the art, and per-application "app packs"

**Date:** 2026-11-05
**Author:** Manoz@Area54, for task #1555 (operator's question to AgentY, 2026-11-05)
**Status:** research only, no code. Uncommitted by design; it ships with the PR that cites it.
**Checked against:** origin/main `dd45d365a` (v0.60.16). Web sources were read on 2026-11-05; where a figure comes from a secondary page it says so.

> [!IMPORTANT]
> **The live timing test was not run.** `desktop:enabled` is off on Area54 and only the person can switch it on (spec §5.7). Section 1 is a code audit, not a measurement. I asked AgentY for the switch; if it is turned on I will add the Notepad run (open, type, save, timed) as an addendum.

## Decisions for the operator

| # | Decision | My recommendation |
|---|---|---|
| D1 | Build the **app pack** system now, on the existing Skills catalog and bundles (no new engine)? | Yes. It is the cheapest step with the most value (§3, Stage 1). |
| D2 | Which three apps first? | Word, Ableton Live, WaveLab: one for each kind of control surface (COM, socket API, UI only). |
| D3 | Add a UI Automation `tree` and `drag` to `Desktop` before anything else? | Yes. It is the biggest accuracy gain for legacy apps (§1, §2). |
| D4 | Let an agent drive **another machine's** desktop over the LAN? | Yes, later, behind the target machine's own switch and per-app grant (§4). Not before D3 ships. |
| D5 | Install community MCP servers for apps (Ableton's runs code inside the app)? | Only through the pack, pinned to a version, and only after the person approves the pack. |

## 1. Where AgentMux is today (from the code, not from memory)

`Desktop` (spec `docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md`, shipped in #5334 and gated in #5677) is **Phase 1 of 5**: Windows only, pixels only.

| Capability | Windows | macOS | Linux |
|---|---|---|---|
| List windows, screenshot one window (`apps`, `look`, scaled to 1568 px) | yes | capture uses `xcap`, which is cross-platform; not tested there, and `Desktop` says Windows only | same |
| Click, scroll, type (500 chars), key chords, wait | yes (`SendInput`, per-monitor DPI aware) | no (`#[cfg(not(windows))]` stub in `desktop_input.rs`) | no |
| Stale-coordinate guard (window moved, covered, DPI changed, point outside) | yes | n/a | n/a |
| **Accessibility tree / UI Automation read** | **no** (spec Phase 2; not built) | no | no |
| Drag, zoom into a region, hold key, multi-monitor choice | **no** | no | no |
| OCR, element search, set-of-marks | **no** | no | no |
| Password-field guard on `type` | **no** (spec Phase 2) | no | no |
| Per-app grant, blocklist, Stop, rate limits, audit log | yes | n/a | n/a |

Every call is gated: off by default, a grant per app asked as a sensitive request (the super agent decides, the person sees a countdown and can veto), browsers, password managers and AgentMux itself refused, 2 inputs a second, every action in `desktop-audit.log`. The gating is stricter than the public descriptions of Cowork, Comet and Operator in the spec's §2 (grant held by AgentMux, audit the agent cannot edit); I did not compare their current versions.

**Across machines: nothing for `Desktop`.** The tool has no `host` argument. What does cross machines today, over LAN peers that pinned the caller's key: `Tasks` (enqueue and list), `Agents` (start, open, stop, set_seat, env), `App` (version, disk, upgrade), and `Identity`; `Message` also reaches agents on LAN peers. `Terminal` reaches WSL and SSH hosts (`connection`). A container agent never gets `Desktop` (no desktop in it; spec §5.7), and the container rule says a refusal needs a host-write reason, which this has.

**Gaps that matter, in order:**
1. No structured read of the app. The agent works from pixels, which is the weakest route for dense legacy UIs (WaveLab, Ableton's mixer).
2. No drag, no zoom: sliders, faders, selections and small text are out of reach or unreliable.
3. Windows only; no remote use.
4. No per-app knowledge: each agent rediscovers an app every session.

## 2. State of the art (late 2026)

| System | What it is | Notes |
|---|---|---|
| **Anthropic computer use** | Client-side tool; current docs list `computer_toolset_20260801` with 17 actions (screenshot, **zoom**, click variants, drag, mouse down/up, scroll, type, key, hold_key, wait). Claude 5.x models; legacy `computer_20251124` for 4.x | [Docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool). I read this page through a summariser: check the version string before building on it. Image limit about 2,576 px long edge. |
| OpenAI CUA / Operator | Pixels in a hosted browser; 38.1% OSWorld at launch (Jan 2025), human 72.4% | [OpenAI](https://openai.com/index/computer-using-agent/) |
| Google | The standalone Gemini 2.5 Computer Use model was shut down on 2026-07-28; computer use is now a built-in tool of the Gemini 3.x Flash family. Browser and mobile first, "not yet optimized for desktop OS-level control" | [Gemini API docs](https://ai.google.dev/gemini-api/docs/models/gemini-2.5-computer-use-preview-10-2025) |
| **Microsoft UFO²** | Windows "AgentOS": a HostAgent plus per-app AppAgents; **UI Automation fused with vision parsing (OmniParser)**; native APIs first (xlwings, win32com, python-pptx), GUI as fallback; knowledge from app help docs, Bing and past runs; speculative multi-action (51% fewer LLM calls); automation in a nested desktop so the user keeps theirs | [UFO² overview](https://microsoft.github.io/UFO/ufo2/overview), [paper](https://arxiv.org/html/2504.14603v2) |
| **CUA-Skill** (Microsoft, Jan 2026) | A library of parameterised skills for common Windows apps, with composition graphs and retrieval; 57.5% on Windows Agent Arena (best of three) | [Project](https://microsoft.github.io/cua_skill/), [arXiv 2601.21123](https://arxiv.org/abs/2601.21123) |
| UI-TARS-2 (ByteDance) | Open native GUI model trained with multi-turn RL: 47.5 OSWorld, 50.6 Windows Agent Arena | [arXiv 2509.02544](https://arxiv.org/abs/2509.02544) |
| Windows-MCP and UIA MCP servers | Community MCP servers that read the UI Automation tree, no OCR | [Windows-MCP](https://github.com/xqdd/Windows-MCP), [uiautomation-mcp](https://github.com/locomorange/uiautomation-mcp) |

**Benchmarks, only where they inform a choice.** On OSWorld-Verified the leading Claude models and Qwen sit at about 85% as of Oct 2026, above the 72% human baseline ([leaderboard](https://yutori.com/leaderboards/osworld-verified.md); an aggregator, so treat the exact order as approximate). That says a good model with plain screenshots already does well on general tasks. It does not say that a legacy pro-audio app works: OSWorld uses ordinary apps. Windows Agent Arena (154 tasks) is the Windows-specific one, and its best results come from agents that **add structure** (UFO², CUA-Skill), not from pixels alone.

**Techniques that work now:**
1. **Structure first, pixels as the fallback.** An app's own API or scripting beats UI Automation, which beats screenshots (UFO², Windows-MCP).
2. **Verify after every act.** Anthropic's guidance: end each batch with a screenshot and check the result.
3. **Zoom or crop** for small text and dense areas; scale screenshots to the model's limit and map coordinates back.
4. **Keyboard first**: shortcuts for dropdowns, menus and scrollbars.
5. **Batch several actions per model call**, stop at the first failure (Anthropic's batch pattern; UFO² speculative actions).
6. **Per-app knowledge** (help docs, past runs, skills) loaded on demand (§3).
7. **A sandbox or separate desktop** for anything risky; per-task scoped credentials. Screenshots are an injection channel, so human confirmation for consequential actions.

## 3. "Template per application": does it exist, and what AgentMux should do

**What exists.** There is no standard, shared "template for Ableton" that makes an agent fast. There are pieces:
- **Agent Skills** (`SKILL.md` with name and description, plus bundled files and scripts, loaded in three levels so only what is needed enters context). Anthropic published it on 2025-10-16 and as an open standard at [agentskills.io](https://agentskills.io/) on 2025-12-18; Codex, Gemini CLI, Copilot and Cursor read the same format ([Anthropic engineering post](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)). It is the right container; nobody ships a per-app pack library in it for desktop apps yet that I could find.
- **CUA-Skill** (Microsoft research): the closest thing to the idea for Windows apps, but a research library, not something to install.
- **App MCP servers**: [AbletonMCP](https://github.com/mcpblender/ableton-mcp) (a Remote Script in Live's embedded Python opens a local TCP socket; one variant `exec()`s Python with the full Live API), Word servers over COM ([WordMcp.McpServer](https://www.nuget.org/packages/WordMcp.McpServer), [word-mcp-live](https://pypi.org/project/word-mcp-live/1.6.5/)) and Microsoft's [Agent 365 Word server](https://learn.microsoft.com/ga-ie/microsoft-agent-365/mcp-server-reference/word) for cloud documents.
- **The app's own scripting**: WaveLab is a COM server with VBScript/JScript control on Windows, plus an ECMAScript script language inside ([Steinberg help](https://archive.steinberg.help/wavelab_pro/v10/extras/wavelab_pro_activex_scripting.html)). Ableton and Word are above.

**Proposal: an "app pack" is an Agent Skill, delivered through what AgentMux already has.**
- A pack is a folder: `SKILL.md` (when to use it, the ladder below), `controls.md`, `keys.md`, `pitfalls.md`, `verify.md`, optional `scripts/` and an optional MCP server entry. It uses the same catalog as today's skills (`Skills` list, upsert, bind, reach), so it reaches an agent at its next launch.
- **Control surfaces, in preference order, written into every pack:** (1) the app's API or scripting, (2) a connector or MCP server, (3) shortcuts and menu paths, (4) UI landmarks for `Desktop` with the verify cue after each step. This matches the ladder already in the tool description.
- **Created** by an agent from the vendor docs plus a recorded exploration session (Anthropic's own advice is to collaborate with Claude to capture successful patterns), then reviewed by a person. **Shared** by riding in a bundle: bundles already export, import and carry skills across installs and LAN peers. **Versioned** with the app version it was verified against in the frontmatter, so a pack can say "checked on Live 12.1".
- **A pack grants nothing.** The per-app `Desktop` grant, the blocklist and Stop still apply. A pack that installs an MCP server needs the person's approval of the pack (D5).

| App | Best surface | Then | Pitfalls to write down |
|---|---|---|---|
| **Word** | COM through a Word MCP server (open, read and edit text, tables, export to PDF); `docx` files directly when the file is not open | Office.js for cloud documents; shortcuts | Modal dialogs block COM; autosave and track-changes state |
| **Ableton Live** | Remote Script plus an MCP server or OSC bridge (create tracks, clips, parameters) | Keyboard map for Session and Arrangement; UI only for the browser panel and plug-in windows | Plug-in UIs are custom-drawn (pixels only); the exec-style server runs arbitrary code in Live |
| **WaveLab** | Its COM/ActiveX scripting and ECMAScript scripts for batch and file work | Shortcuts and the Master Section by keyboard; `Desktop` clicks with zoom for meters and clip editing | No macro recorder; most editing is UI; long renders need wait-and-verify loops |

## 4. Across machines and tiers

Today the pieces exist but are not joined: LAN peers that pinned each other's key can run `Agents start/open/stop` and `Tasks` on each other, and every cross-machine call is recorded as the agent that made it. A remote `Desktop` would add:
- **Consent stays at the target.** The target machine's own `desktop:enabled`, per-app grant and Stop decide; the caller's machine never grants on its behalf. The sensitive-request countdown shows on the target.
- **Latency.** A look-act-verify loop sends one 1568 px screenshot per step. On a LAN that is a fraction of a second; over the cloud relay it is seconds and costs more. Prefer sending the app's text (UI Automation tree, a COM or script result) over images when the route is slow: another reason for D3 first.
- **Who may drive whom.** A caller must be a pinned peer (as for `Agents host`), the target's audit log records the caller by name and host, and rate limits stay per grant.
- **Tiers.** A container agent still cannot hold `Desktop`, but it can ask a host agent to run an app pack's script, which is the same rule as for any other host-write.
- **Alternative to consider:** a separate desktop (UFO²'s nested desktop, or a VM) for risky or long runs. It removes the "user and agent fight for the mouse" problem; the cost is a second Windows session per run.

## 5. Recommendation and rough cost

My estimates, not measured; one agent, in order of value per cost.

| Stage | What | Rough cost |
|---|---|---|
| 1 | **App packs on the Skills catalog**: a pack template and a how-to-write-one skill; Word, Ableton and WaveLab packs; one live test per pack | 3 to 5 days, no Rust |
| 2 | **`Desktop` v2 (Windows)**: `tree` (UI Automation, with names, roles and bounds), `drag`, `zoom`, password-field guard; a settings switch for the structured read | 1 to 2 weeks |
| 3 | **Remote Desktop over the LAN**: a `host` argument, target-side consent and audit, text-first replies | 1 to 2 weeks plus a security review |
| 4 | **macOS then Linux X11** input (`CGEvent` and the Accessibility permission; `XTest`) | 1 to 2 weeks each |
| 5 | Pack authoring from a recorded session; a pack library in the bundle store | Decide after Stage 1 shows what packs need |

**Decide first (D1 to D5 above).** Stages 1 and 2 do not depend on each other and can run in parallel.

## Sources

- [Anthropic computer use tool docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool) · [Agent Skills engineering post](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills) · [agentskills.io](https://agentskills.io/)
- [OpenAI CUA](https://openai.com/index/computer-using-agent/) · [Gemini computer use docs](https://ai.google.dev/gemini-api/docs/models/gemini-2.5-computer-use-preview-10-2025)
- [UFO²](https://microsoft.github.io/UFO/ufo2/overview) · [UFO² paper](https://arxiv.org/html/2504.14603v2) · [CUA-Skill](https://arxiv.org/abs/2601.21123) · [UI-TARS-2](https://arxiv.org/abs/2509.02544)
- [OSWorld-Verified leaderboard (aggregator)](https://yutori.com/leaderboards/osworld-verified.md)
- [Windows-MCP](https://github.com/xqdd/Windows-MCP) · [uiautomation-mcp](https://github.com/locomorange/uiautomation-mcp)
- [AbletonMCP](https://github.com/mcpblender/ableton-mcp) · [WordMcp.McpServer](https://www.nuget.org/packages/WordMcp.McpServer) · [word-mcp-live](https://pypi.org/project/word-mcp-live/1.6.5/) · [Agent 365 Word](https://learn.microsoft.com/ga-ie/microsoft-agent-365/mcp-server-reference/word) · [WaveLab ActiveX scripting](https://archive.steinberg.help/wavelab_pro/v10/extras/wavelab_pro_activex_scripting.html)
- In the repo: `docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md`, `crates/mcp/src/tools/desktop.rs`, `crates/mcp/src/desktop_input.rs`, `crates/srv/src/server/app_api/desktop.rs`, `schema/settings.json` (`desktop:*`)

Agent: Manoz@Area54
