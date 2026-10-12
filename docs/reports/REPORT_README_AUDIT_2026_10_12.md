# REPORT — README audit against the code, and file paths as links

**Date:** 2026-10-12
**Status:** analysis — findings and a plan; the README rewrite itself is a follow-up PR.
**Author:** AgentA (Area54)
**Trigger:** Owner request: "we need to update the agentmux project readme with all sorts of updates, including the widget icons which changed since font awesome free, there are also more widgets. lets do an audit of the code/comments/docs .. did the work regarding fixing broken comment links get finished?" and then "lets ensure all file paths are also hyperlinks in any md files, so users can quickly peruse documentation."
**Method:** Every factual claim in `README.md` checked against `main` at `556412186` (widgets, providers, panes, settings, agents, jekt markers, App API, build, tasks, crates, CEF fork, changesets, releases). The four largest "wrong" verdicts (ACP, rename, account switching, Settings sections) were re-checked by hand. Links checked with [`scripts/check-doc-links.mjs`](../../scripts/check-doc-links.mjs), comment references with `node scripts/check-comment-hygiene.mjs --dead-refs`, and every external README link with an HTTP request.

## 1. Summary

- **Comment and doc links: done.** The dead-file-reference work in comments (#4251, #4255, #4259, #4270, #4273, #4305, #4587) took the count from 162 to 0, and the gate keeps it there. On `556412186` there is one new bare name ([`scripts/ui-screenshots/install-fake-claude.mjs:38`](../../scripts/ui-screenshots/install-fake-claude.mjs#L38), an example, harmless). Relative Markdown links: 0 broken of 1,462 under [`docs/`](..). Across every `.md` in the repo there was one broken link, in [`scripts/benchmarks/README.md`](../../scripts/benchmarks/README.md) (a Tauri-era `docs/TAURI_MIGRATION_STATUS.md`), removed in this PR.
- **README: about 55 of ~90 claims hold; the rest are stale or wrong.** The widget table is the most visible: it lists 9 of 22 widgets, three of its icon names are Font Awesome Pro names renamed in the move to Free (#4665), and "every widget is pinned by default" stopped being true when unpinned widgets arrived.
- **File paths are now links.** A new script, [`scripts/link-md-paths.mjs`](../../scripts/link-md-paths.mjs), turns a backticked repo path into a relative link. This PR applies it to the top-level docs (34 paths) and widens [`scripts/check-doc-links.mjs`](../../scripts/check-doc-links.mjs) from [`docs/`](..) to every tracked `.md`, so README links are checked too. About 11,100 more unlinked paths sit in [`docs/`](..) (8,100 in [`docs/specs/`](../specs)); those get their own PR so this one stays reviewable.

## 2. Widgets (`crates/srv/src/config/widgets.json`)

How an icon renders: `makeIconClass()` in [`frontend/util/util.ts`](../../frontend/util/util.ts) turns `name`, `solid@name`, `regular@name` or `brands@name` into Font Awesome classes. Font Awesome Free 6.7.2 is vendored in [`public/fontawesome/`](../../public/fontawesome), and [`public/fontawesome/css/pro-icon-aliases.css`](../../public/fontawesome/css/pro-icon-aliases.css) maps every Pro-only name to a Free glyph, so an old name still renders, just not as its own icon.

The move to Free was #4665 (2026-10-11). Renames in `widgets.json`: `sparkles` → `wand-magic-sparkles` (Agent), `bee` → `people-group` (Swarm), `square-terminal` → `terminal` (Terminal).

| Widget | View | Icon | Default place |
|---|---|---|---|
| Agent | `agent` | wand-magic-sparkles | widget bar |
| Swarm | `swarm` | people-group | widget bar |
| Memory | `memory` | brain | widget bar |
| Hangar | `files` | folder-open | widget bar |
| Connectors | `connectors` | plug | widget bar |
| Terminal | `term` | terminal | widget bar |
| Editor | `editor` | file-code | widget bar |
| Browser | `browser` | globe | widget bar |
| Messengers | (group) | comments | widget bar; holds Discord, Slack, Telegram, WhatsApp, Teams (browser panes, `brands@` icons) |
| Sysinfo | `sysinfo` | chart-line | widget bar |
| Help | `help` | circle-question | widget bar |
| Drone | `drone` | diagram-project | More menu |
| Warden | `warden` | shield-halved | More menu |
| Media | `media` | photo-film | More menu |
| Toolchain | `toolchain` | wrench | More menu |
| Tower | `tower` | tower-observation | More menu |
| Settings | `settings` | cog | More menu |

The bar narrows in three steps ([`frontend/app/window/use-widget-bar-responsive.ts`](../../frontend/app/window/use-widget-bar-responsive.ts)): labels, icons only, then icons plus a "more" button. The pane "+" picker lists every widget regardless of pinning (`buildPaneWidgetMenuItems` in [`frontend/app/window/action-widgets-config.ts`](../../frontend/app/window/action-widgets-config.ts)).

## 3. README claims that are wrong

| README | Claim | What the code does |
|---|---|---|
| What AgentMux Does | Most CLIs run per turn via Subprocess/ACP; only Claude is persistent | ACP keeps one long-lived process ([`crates/srv/src/backend/blockcontroller/acp.rs`](../../crates/srv/src/backend/blockcontroller/acp.rs)). Copilot, OpenClaw and Pi are long-lived like Claude; Codex, Gemini, Qwen, Kimi, Mux Code and Antigravity run per turn. |
| Accounts; Agent identity | Accounts swap without a restart | Binding an account restarts the agent ([`frontend/app/view/identity/bind-to-agent-menu.ts`](../../frontend/app/view/identity/bind-to-agent-menu.ts), `forcerestart: true`). |
| How to rename an agent | Hover a card, click the pencil | No pencil on [`frontend/app/view/agent/components/AgentCard.tsx`](../../frontend/app/view/agent/components/AgentCard.tsx). Rename is in the My Agents row menu and on the pane tab. |
| Agent identity (accounts) | A 👤 button on the agent card | No such button. Accounts are set in the Stash (Accounts tab) or in Connectors → Accounts. |
| Not widgets → Settings | Sections: Appearance, Window & Panes, Terminal, Sounds, Network, Advanced | No Network section. Ten sections: Appearance, Window & Panes, Browser, Terminal, Sounds, Notifications & Tray, Recording, Paired devices, Widgets, Advanced ([`frontend/app/view/settings/settings-view.tsx`](../../frontend/app/view/settings/settings-view.tsx)). Settings is also a widget now. |
| Build Commands | `task test` | No such task; tests run with `npm test`. |
| Production Build | `task package` builds for the host platform | Windows only (`platforms: [windows]` in `Taskfile.yml`); macOS is `task package:macos`, Linux `task package:linux`. |
| Architecture | srv is auto-spawned by the host | The launcher spawns srv ([`crates/launcher/src/srv_spawner.rs`](../../crates/launcher/src/srv_spawner.rs)); the host does it only as the `task dev` fallback. |

## 4. README claims that are out of date

- **Providers:** ten, not eight; Mux Code and Antigravity are missing ([`crates/srv/src/backend/providers.rs`](../../crates/srv/src/backend/providers.rs)).
- **Crates:** eight, not five; [`crates/bashwrap`](../../crates/bashwrap), [`crates/mcp`](../../crates/mcp), [`crates/remote`](../../crates/remote) and [`crates/procstats`](../../crates/procstats) are not mentioned.
- **Stash:** seven tabs (adds Registration and Devices), and it is a drawer under the pane header now, not a modal.
- **Jekt marker:** carries `TO=`, `HELD_FOR=`, `INSTANCE=`, `SIG=`, `ESCALATE=` and `PRIORITY=` as well; the trust table lacks `wan-verified`, and `SIG=` is its own field, not a `TRUST=` value.
- **CEF fork:** Chromium 154 on integration branch `8037`; five changes across 20 source files ([`docs/cef-build/CEF_FORK_MAINTENANCE.md`](../cef-build/CEF_FORK_MAINTENANCE.md)); pins live in two files that must move together, [`scripts/cef-build/cef-runtime-pins.sh`](../../scripts/cef-build/cef-runtime-pins.sh) and [`scripts/cef-build/windows-runtime-pin.sh`](../../scripts/cef-build/windows-runtime-pin.sh).
- **Releases:** Linux also ships `.deb`, `.rpm` and `.tar.gz`; the 7-day nightly retention applies to Windows artifacts only; macOS release builds depend on a CEF-availability check.
- **Prerequisites:** Node 24.11 or later (`.nvmrc`); the Linux build-dependency link should go to `BUILD.md`, not [`docs/linux.md`](../linux.md), and the apt list is a subset of `BUILD.md`'s.
- **Hamburger menu, Swarm, App API:** all correct but undersold (the menu has 13 items; Swarm also shows background tasks, todos and remote fleets; the App API has about 250 commands).
- **Linked App API docs:** [`docs/api/getting-started.md`](../api/getting-started.md) still calls `pane.open` a draft, and [`docs/specs/app-api-status.md`](../specs/app-api-status.md) dates from April.

## 5. Features the README doesn't mention

Hangar, Tower, Media, Toolchain and Messengers widgets; the widget SDK and user widgets ([`sdk/widget-sdk/`](../../sdk/widget-sdk)); LAN discovery and MuxBus cloud; paired devices (mobile viewer); the work queue and cron; the agentmux MCP server ([`crates/mcp`](../../crates/mcp)); container agents and headless srv; SSH and WSL remotes; bundle import/export; voice input and recording; the command palette.

## 6. Claims that need an owner decision

- **Memory and size figures.** "150–350 MB, stays flat" and "~160 MB ZIP, ~150 ms startup, 150–350 MB resident" have nothing behind them in the repo. A fresh idle v0.59.20 on Windows measured about 540 MB private memory in the #4712 pilot and 740 MB private commit (median of three) in #4714, and the Windows portable is a `.7z` of about 127 MiB (#4705).
- **"No telemetry, no phone-home."** Nothing sends telemetry. Some features do reach the network when the user turns them on: MuxBus cloud, voice transcription and provider CLI installs.

## 7. Plan

1. **This PR:** this report; [`scripts/link-md-paths.mjs`](../../scripts/link-md-paths.mjs) with tests; paths linked in the top-level docs; [`scripts/check-doc-links.mjs`](../../scripts/check-doc-links.mjs) checks every tracked `.md`; the one broken link removed.
2. **Next PR:** run `node scripts/link-md-paths.mjs --all` over [`docs/`](..) and the remaining `.md` files.
3. **README rewrite:** fix every item in §3 and §4, rebuild the widget table from `widgets.json`, add short sections for §5 that link to docs.agentmux.ai, and settle §6.
