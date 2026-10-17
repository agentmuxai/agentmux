# SPEC: Tower in the theme's colours, with process types you can see

**Status:** active — built on branch `loap/tower-process-types`, under test
**Date:** 2026-10-17
**Requested by:** the operator:
- "write a spec to add more color to the tower. It also needs to more closely respect the color theme, it is hard black background throughout all the dark themes, which is wrong. have it follow a look similar to hangar"
- then: "we want proactive intelligent use of colors, for example to categorize process types"

**Author:** Loap@narko
**Builds on:** `SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md` and `SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md` (the Tower), `SPEC_FILE_BROWSER_PANE_2026_10_01.md` (the Hangar, the look to follow)
**Related:** `SPEC_COLOR_THEME_TOKEN_HARDENING_2026_09_21.md` (theme tokens only, never literal colours in components), `SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12.md` (what `--main-bg-color` carries), `SPEC_LIGHT_THEME_RULES_AND_CONTRAST_ENFORCEMENT_2026_10_01.md`

---

## 0. The ask

1. **The surface.** The Tower's background is black in the dark themes. It should be the theme's own colour, like the Hangar's.
2. **Colour with a job.** The Tower is nearly all grey. Colour should tell the user something: first of all what kind of process each one is, and then which agent's work it is and how heavy it is.

Part 1 (§1, §5) fixes the surface. Part 2 (§2–§4) gives processes a type and spends colour on it.

## 1. What is wrong now

### 1.1 The black

A pane's body is painted by its frame: `.block-frame-default-inner` sets `background-color: var(--block-bg-color)` (`frontend/app/block/block.scss`). In every theme that is a darker (or, in light themes, lighter) translucent layer than the theme's surface, `--main-bg-color`. The Hangar paints its own surface (`.files-view { background: var(--main-bg-color) }`); `.tower-view` paints none, so it shows the frame.

Measured 2026-10-17 in an isolated dev build of 0.59.30, with the Tower and the Hangar side by side and each theme applied in turn. "Tower body" is the frame composited over the window, what the eye sees. The Tower's sticky table header paints `--main-bg-color`, so it matches the Hangar, not the body under it:

| Theme | Hangar surface and Tower header | Tower body |
|---|---|---|
| Default dark | rgb(34, 34, 34) | rgb(17, 17, 17) |
| Dracula | rgb(40, 42, 54) | rgb(29, 30, 39) |
| Nord | rgb(46, 52, 64) | rgb(35, 39, 49) |
| Tokyo Night | rgb(26, 27, 46) | rgb(18, 19, 32) |
| Catppuccin | rgb(30, 30, 46) | rgb(21, 21, 33) |
| Gruvbox | rgb(40, 40, 40) | rgb(26, 26, 26) |
| Monokai | rgb(39, 40, 34) | rgb(30, 31, 26) |
| Midnight | rgb(10, 12, 22) | rgb(6, 8, 16) |
| High Contrast | rgb(0, 0, 0) | rgb(0, 0, 0) |
| Light | rgb(246, 246, 247) | rgb(251, 251, 252) |
| Catppuccin Latte | rgb(239, 241, 245) | rgb(249, 249, 251) |

So:
- **The black is the default dark theme** (rgb(17, 17, 17)), with Midnight close behind.
- **The named dark themes** show a shade 20 to 30% darker than their own colour, which reads as black beside any pane that paints the real one.
- **The sticky header is a lighter band** across the table.
- **Light themes have the same bug** the other way round.
- **High Contrast** is black by design and stays black.

### 1.2 The grey

What colour the Tower has today:
- **Agent identity:** agent swatches and CPU sparklines in the Agents view, and a 3 px bar in each agent's colour in the list grouped by agent.
- **Errors:** shown in red.
- **Charts:** both series take the agent's colour, so they are the same hue. An agent's colour is a fixed hex that ignores the theme, and in the dev build a dark-olive agent's chart was a faint sliver.

Everything else is `--main-text-color` or `--secondary-text-color`.

The Tower knows much more than it shows. Each process arrives with:
- its executable `name`;
- its part in its task, `role` (`main`, `support`, `started`);
- the task's `kind` (`agent`, `terminal`, `agentmux`);
- a `detail` for AgentMux's own processes ("GPU", "Renderer");
- `started_by`, the agent tool call that launched it.

None of it is used to tell a compiler from a browser from a system service. On this machine, the operator's, the full list is dominated by system services (97 `svchost`), consoles (61 `conhost`) and browsers (48 `chrome`, 18 WebView2), with builds and agents scattered among them.

## 2. Principles

1. **One channel, one meaning.** Each kind of colour appears in one place and means one thing, so the reader never has to guess which meaning a colour has:

   | Channel | Means | Where |
   |---|---|---|
   | Type icon, in the type's colour | what kind of process | before the name |
   | Owner bar and owner wash | which agent's work it is | the row's left edge, the owner's header row |
   | Bar in the CPU and Memory cells | how much of the machine it takes | the number columns |
   | Warning or error colour on a figure | it is high for its scope | the number columns |
   | Accent | the UI's own state: selected, sorted, focused | chrome |
   | Muted text | background: OS processes, support processes, exited ones | the whole row |

2. **Never colour alone.** Every type has an icon, and a high figure is bold as well as coloured, so the Tower reads in greyscale and for colour-blind users.
3. **Spend colour on what the user acts on:** their agents and the work those agents run. The OS and its services get muted text, not a colour.
4. **Colours come from the theme and are measured.** Each theme gets the five type colours it can keep apart (§4), drawn from its own palette.
5. **Red is never a type.** It means trouble.

## 3. Process types

### 3.1 The types

| Type | What's in it | Examples | Icon | Colour |
|---|---|---|---|---|
| **Agents & AI** | agent CLIs (every provider's `cli_command`), the main process of an agent pane, local model servers | `claude`, `codex`, `gemini` (a `node` running its package), `opencode`, `aider`, `goose`, `ollama`, `llama-server`, `lmstudio` | the provider's logo where known (`ProviderLogo`), else `fa-robot` | `--process-kind-agents` |
| **AgentMux** | AgentMux's own processes | `agentmux-cef`, `agentmux-srv-…`, `agentmux-mcp`, `agentmux-bashwrap`, `agentmux-launcher`, the remote helper | AgentMux's own icon (`ProviderLogo` `agentmux`) | `--process-kind-agentmux` |
| **Shells & commands** | shells, consoles, terminal hosts, remote sessions, and the command-line tools run in them | `bash`, `zsh`, `fish`, `pwsh`, `cmd`, `conhost`, `OpenConsole`, `tmux`, `ssh`, `grep`, `rg`, `sed`, `head`, `tail`, `jq`, `curl` | `fa-terminal` | `--process-kind-shells` |
| **Builds & tools** | compilers, linkers, build systems, package managers, test runners, version control | `cargo`, `rustc`, `cl`, `link`, `clang`, `gcc`, `make`, `cmake`, `ninja`, `msbuild`, `tsc`, `esbuild`, `go`, `gradle`, `npm`, `pnpm`, `pip`, `uv`, `git`, `gh`, `task` | `fa-hammer` | `--process-kind-builds` |
| **Runtimes & servers** | interpreters running a program, language servers, databases, local web servers | `node`, `python`, `java`, `dotnet`, `deno`, `bun`, `ruby`, `rust-analyzer`, `gopls`, `clangd`, `postgres`, `redis-server`, `nginx` | `fa-server` | `--process-kind-runtimes` |
| **Browsers & web views** | browsers and embedded web views | `chrome`, `msedge`, `firefox`, `safari`, `brave`, `msedgewebview2`, "Google Chrome Helper (Renderer)" | `fa-globe` | text colour |
| **Containers & VMs** | container engines, WSL, virtual machines | `docker`, `com.docker.*`, `containerd`, `podman`, `wsl`, `wslhost`, `vmmem`, `vmmemWSL`, `vmwp`, `vmcompute`, `qemu-*`, `VirtualBox*`, `vmware-*` | `fa-cube` | text colour |
| **Editors & IDEs** | code editors and IDEs | `Code`, `cursor`, `zed`, `idea64`, `devenv`, `nvim`, `vim`, `emacs`, `sublime_text` | `fa-code` | text colour |
| **Apps** | everything else the user runs | `Discord`, `KeePass`, `Spotify` | `fa-window-maximize` (regular) | text colour |
| **System** | the OS, its services, security software, device drivers | Windows: `svchost`, `csrss`, `lsass`, `dwm`, `explorer`, `RuntimeBroker`, `MsMpEng`, `nvcontainer`, `Rtk*`, `AMD*`. macOS: `kernel_task`, `launchd`, `WindowServer`, `mds*`. Linux: `systemd*`, `kworker/*`, `Xorg` | `fa-microchip` | muted |

The five coloured types are the work an agent runs or is: agents, AgentMux itself, shells, builds and the runtimes they start. Browsers and VMs are often the biggest consumers on a machine, but they are seldom what the user is checking on in AgentMux, and every theme can keep only five colours apart (§4). They get an icon, and the breakdown bar (§3.3) still shows how much they take.

### 3.2 How a process gets its type

srv decides, in this order. The first rule that matches wins.

1. **AgentMux's own:** a task of kind `agentmux`, a process with a `detail`, or an AgentMux name.
2. **The main process of an agent pane:** Agents & AI, with the pane's provider id, so the row can show its logo whatever the executable is called.
3. **The main process of a terminal pane:** Shells & commands.
4. **The name, looked up in one table.** The name is lower-cased, and `.exe` and version suffixes are stripped: `agentmux-srv-0.59.30-windows.x64` reads as `agentmux-srv`, `python3.12` as `python`. The table is the providers' `cli_command`s, taken from `all_providers()` so a new provider needs no edit here, plus a built-in list in `backend/tower_kind.rs`. Some entries match a prefix (`com.docker.`, `kworker/`, `google chrome helper`).
5. **The command line, for a generic runtime only, locally only.** A `node`, `python`, `java`, `dotnet`, `deno`, `bun` or `ruby` inside an agent or terminal task is refined by its command line:
   - npm, pnpm, yarn, vite, webpack, tsc, jest, vitest, pytest or playwright makes it **Builds & tools**;
   - an agent CLI's package makes it **Agents & AI**. The packages are the providers' own `npm_package` and companion packages from the registry (`@anthropic-ai/claude-code`, `@google/gemini-cli`, `@openai/codex`, …), so a new provider needs no edit here either;
   - anything else stays **Runtimes & servers**.

   The command line is read once per process, cached by its process key, and never leaves srv. Only the type is sent: command lines can carry tokens.
6. **Anything else:** Apps.

**Another machine's** frames carry names only, by design, so srv classifies them by name (rule 4) when they arrive. The remote helper doesn't change.

**On the wire,** `TowerProcess` gains:
- `kind: Option<TowerProcessKind>`: `agents`, `agentmux`, `shells`, `builds`, `runtimes`, `browsers`, `containers`, `editors`, `apps`, `system`;
- `provider: Option<String>`, on an agent's main process only.

An older srv sends neither, and the row shows no type icon. The frontend never guesses from names, so there is one table to keep right, in Rust, with its tests.

A **support** process (an agent's MCP servers, a conhost started for it) gets its type icon, but its row stays muted, as today.

### 3.3 Where the types show

- **The icon.** Every process row, in every view, starts with its type's icon in the type's colour. A group row (`chrome ×24`) uses its processes' type.
- **The breakdown.** At the top of the Processes view: one stacked bar for CPU and one for memory, a segment per type.
  - The five coloured types use their colours.
  - The four icon-only types use graded tints of the text colour (`--tint-*`, strongest for Browsers).
  - System uses the faintest tint.

  A segment's tooltip reads "Browsers & web views · 24 processes · 18% CPU · 3.1 GB". This answers "where is my machine going?" before the list is read.
- **Type chips.** Under the breakdown, one chip per type present, each with its icon, its count and its share of CPU. Clicking a chip filters the list to that type; clicking it again clears the filter. The filter is kept in the pane's meta, like the sort.
- **Group by type.** The Processes view's grouping setting (`tower:group`: `agent`, `app` or `none`) gains `type`. Each type becomes a header row with its icon, its colour as a 3 px bar, and its totals.
- **System collapses.** In the flat list and the list by app, System processes start folded into one "System" row with totals, unless System is the type chosen with a chip. On Windows they are half the list. One click opens it, and it stays open while the pane is, like any group.
- **Owner continuity.** In the flat process list, a process that belongs to an agent's task (`task`) gets the 3 px owner bar in that agent's colour, as the grouped list already shows. So "which agent" is visible in every view, beside "what kind".

## 4. The colours

### 4.1 Five tokens

`theme.scss` gains five tokens. Each is an alias of a terminal colour, because every theme already tunes its terminal palette to read on its own background:

```scss
--process-kind-agents: var(--term-magenta);
--process-kind-agentmux: var(--term-cyan);
--process-kind-shells: var(--term-green);
--process-kind-builds: var(--term-yellow);
--process-kind-runtimes: var(--term-blue);
```

Components use only these tokens, never `--term-*` directly, so a theme can retune one type without touching the terminal. The names say "process", not "tower", because any pane that shows processes (Swarm, Sysinfo) can use them.

### 4.2 Why five, and the measurements

The terminal palette looks like 12 colours, but it isn't. Measured from the theme files (OKLab distance ×100; about 2 is barely visible, and at least 9 is needed to tell two icons apart at a glance):

- **Bright duplicates.** In Catppuccin, Monokai, Nord and Tokyo Night, `--term-blue` and `--term-bright-blue` are the same colour (distance 0).
- **Base collisions.** Monokai's blue and cyan are both `#66d9ef`. Catppuccin's green and cyan are 7.0 apart, Gruvbox's 8.2.
- **Low contrast.** Several colours fall below the 3:1 a non-text icon needs against its background (WCAG 1.4.11): blue on the default dark theme (2.68:1), yellow on Catppuccin Latte (2.31:1).

Red is reserved for trouble, which leaves five hue families. With the defaults above and these overrides, each drawn from the theme's own palette, all 13 themes pass both checks:

| Theme | Overrides | Closest pair | Lowest contrast |
|---|---|---|---|
| Default dark | AgentMux `--term-bright-cyan`, Runtimes `--term-bright-blue` (blue is 2.68:1) | 16.0 | 3.47:1 |
| Dracula | none | 15.9 | 3.03:1 |
| High Contrast | none | 16.9 | 4.13:1 |
| Light | none | 13.1 | 4.51:1 |
| Midnight | AgentMux `--term-bright-cyan` | 13.4 | 6.03:1 |
| Gruvbox | AgentMux `--term-bright-cyan`, Builds `--term-bright-yellow` | 11.9 | 3.48:1 |
| Monokai | Runtimes `#fd971f`, Monokai's orange (its blue and cyan are one colour) | 11.0 | 5.23:1 |
| Catppuccin | AgentMux `#89dceb` (sky), Runtimes `#b4befe` (lavender) | 9.9 | 9.17:1 |
| Tokyo Night | Runtimes `#9d7cd8`, its purple | 9.6 | 5.09:1 |
| Nord | Runtimes `#5e81ac`, its dark frost | 9.5 | 3.10:1 |
| Gruvbox Light | Shells `--term-bright-green` | 9.9 | 3.24:1 |
| Solarized Light | Builds `--term-bright-red` (Solarized's orange); all five `color-mix(in oklab, …, var(--main-text-color) 5%)` | 11.9 | 3.05:1 |
| Catppuccin Latte | all five `color-mix(in oklab, …, var(--main-text-color) 20%)` | 12.6 | 3.04:1 |

The overrides are literal colours in theme files, which is where a theme's colours belong. Components still use only the tokens.

Catppuccin's best pair without overrides used its peach, which is also its `--warning-color`. Sky and lavender are chosen instead, at 9.9, so a type never shares the warning colour there. In some themes, Builds' yellow equals `--warning-color` (Monokai, Nord, Tokyo Night). That is allowed by principle 1: a type colour appears only on the icon before the name, a warning only on a figure.

### 4.3 The guard

`frontend/app/view/tower/process-kind-colours.test.ts` reads every theme file and resolves the five tokens: `var()` to the terminal colours, hex, and `color-mix(in oklab, …)`. For each theme it asserts:
- every pair of the five is at least 9.0 apart (OKLab ×100);
- each has at least 3:1 contrast against `--main-bg-color`.

A new theme, or a palette edit that breaks either, fails CI and names the theme and the pair. The measurement scripts behind §4.2 become this test.

## 5. Surface, chrome, charts and load

### 5.1 Surface

`.tower-view` gains `background: var(--main-bg-color)`, exactly as `.files-view` has it. The sticky header then matches the body. The table, toolbar and rail stay transparent over it. The chart tooltip and the pairing panel keep `--modal-bg-color`. `--main-bg-color` carries `--window-opacity`, so a translucent window shows through the Tower as it does through the Hangar. This is phase 1 on its own, a one-line change.

### 5.2 Chrome like the Hangar

| Element | Today | Becomes |
|---|---|---|
| Rail row selected | `--hover-bg-color` plus a 2 px accent edge | `--highlight-bg-color`, edge kept |
| Sorted column header | the sort button's default colour | `--accent-color` for its label and arrow |
| Rail fixed entries' icons | secondary text | accent for the selected one, as `.files-place-active i` |
| Hover | `--hover-bg-color` | unchanged |

Rail rows already show a focus ring, and table rows aren't focusable.

### 5.3 Charts and totals

- **Series colours.** CPU uses `--sysinfo-cpu-color` and Memory `--sysinfo-mem-color`, the Sysinfo pane's colours for the same two series.
- **The agent's colour** stays on CPU, so the chart matches the agent's rail sparkline. The area fill moves from 15% to 20% opacity.
- **Totals.** In the summary line and the agent detail header, the CPU figure takes the CPU colour and the memory figure the memory colour. Their labels stay muted.

### 5.4 Load

- **The bar.** Each CPU and Memory cell gets a thin bar under its figure. Its length is the process's share of the machine, and it is drawn in the row's type colour at 35% (text-colour tint for the icon-only types). The number column becomes a bar chart of what is taking the machine, by type.
- **The level.** A figure that is high for its scope turns `--warning-color`, then `--error-color`, and bold. The thresholds live in one function, `loadLevel(kind, scope, fraction)` in `tower-util.ts`:

  | Figure | Scope | Warning at | Error at |
  |---|---|---|---|
  | CPU | one process, group or agent, share of the machine | 25% | 50% |
  | CPU | the machine | 60% | 85% |
  | Memory | one process, group or agent, share of installed memory | 10% | 25% |
  | Memory | the machine | 70% | 90% |

- **Installed memory.** It isn't in the Tower's sample today (`TowerSnapshot` carries `cpu_count`, not memory). `TowerMachine` gains `mem_total`. srv already reads it for the status bar (`backend/sysinfo.rs`, `sys.total_memory()`). Another machine's frames don't carry it, so their memory figures get bars but no level until the remote helper sends it too.

## 6. Where it goes

| Where | Change |
|---|---|
| `crates/srv/src/backend/tower_kind.rs` (new) | the type table and `classify(name, role, task kind, detail, provider, command line)`; agent CLIs and their packages from `providers::all_providers()` |
| `crates/srv/src/backend/rpc_types/tower.rs` | `TowerProcessKind`; `TowerProcess.kind`, `.provider`; `TowerMachine.mem_total`; regenerated TS types |
| `crates/srv/src/backend/tower_sampler.rs` | classify each process; the command-line cache for runtimes in tasks |
| `crates/srv/src/backend/tower_remote.rs` | classify a remote frame's names on arrival |
| `frontend/app/theme.scss`, 9 theme files | the five tokens and §4.2's overrides |
| `frontend/app/view/tower/tower-kinds.ts` (new) | type → label, icon, token, order |
| `tower-process-name.tsx` | the type icon (provider logo for agents) |
| `tower-view.tsx`, `tower-agents.tsx` | breakdown, chips and filter, group by type, System folded, owner bar in the flat list, load bars |
| `tower-util.ts` | per-type totals, `loadLevel` |
| `tower-history.tsx` | series colours |
| `tower-view.scss` | surface, chrome, icon colours, bars, levels |

## 7. Phases

1. **Surface** (§5.1), with the chrome (§5.2). Small, and it fixes the black on its own.
2. **Types.** The srv classifier and wire fields, the five tokens and their guard test, and the icon on every row.
3. **Using the types.** The breakdown, chips and filter, group by type, System folded, and the owner bar in the flat list.
4. **Load and charts.** `mem_total`, the load bars and levels, the series colours and totals.

## 8. Tests

1. **Rust (`tower_kind.rs`):**
   - names from each OS, upper and lower case;
   - `.exe` and version-suffix stripping;
   - the AgentMux family, prefix entries, and every provider's `cli_command`;
   - the main process of agent and terminal panes;
   - command-line refinement of `node` and `python` to Builds and to Agents;
   - the command line appears in no wire field;
   - a remote frame classified by name.
2. **The colour guard (§4.3)** for all 13 themes.
3. **`tower-util.test.ts`:** `loadLevel` at, just under and just over each threshold; per-type totals add up to the machine.
4. **`tower-view.test.tsx`:**
   - each type's icon and colour class;
   - an agent's main process shows its provider logo;
   - a chip filters and clears;
   - group by type has one header per type present;
   - System starts folded, and its state is kept;
   - the owner bar appears in the flat list;
   - a busy row has the level class and an idle one doesn't.
5. **Visual, in an isolated dev build,** for every theme in §1.1, with the Tower beside the Hangar:
   - the body matches the Hangar's surface;
   - the header band is gone;
   - each type's icon stands apart;
   - the breakdown reads at a glance;
   - the same holds with the window translucent, and under a deuteranopia simulation, where the icons alone must still tell the types apart.

## 9. Open questions

1. **Which five types get colour.** This spec gives colour to the agent work (Agents, AgentMux, Shells, Builds, Runtimes). Browsers or VMs could take a slot instead, at the cost of one of those.
2. **System folded by default.** Proposed because system processes are half the Windows list. Say if you'd rather it start open.
3. **Thresholds** (§5.4). They are first guesses, kept in one function so they're easy to tune.
4. **Other panes.** The Editor and CPU panes also paint no surface and show the dark frame. This spec fixes the Tower only.
5. **Agent colour on the Memory chart.** Today both series take the agent's colour. This spec keeps it on CPU only, to match the sparkline.
