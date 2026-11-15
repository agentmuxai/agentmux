# SPEC: say, in the new-agent pane and the docs, that AgentMux manages its own agent CLIs

**Status:** active. §4.1 (the note in the agent picker) lands with the inline new-agent PR (`SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md` Part B); the Toolchain part of §4.4 Step A with #5130; §4.2 in agentmux-docs. §4.4 Step B (a one-click run of vendor scripts) is not planned yet.

Written 2026-10-18 (AgentX@narko) at the operator's request:
"in the new harness/agent pane, put a note that agentmux manages its own CLIs, isolated from the system, also verify
it, and add it in the new agent page in agentmux-docs too."

## 1. What the operator asked for

1. A short note in the pane where a user picks the harness for a new agent (the "New Agent" tier of the agent picker)
   saying AgentMux manages its own agent CLIs, separate from the ones installed on the system.
2. Verify that this is true before saying it.
3. The same note on the new-agent page of agentmux-docs.

## 2. Verification (done 2026-10-18, against `main` at 64543db51)

**Verdict: true, with caveats.** It holds for the 13 npm-managed providers (claude, cline, codex, copilot, crush, dsh,
gemini, kilo, muxcode, openclaw, opencode, pi, qwen). It does not hold for six providers that AgentMux cannot install,
and "isolated" means the CLI binary and its login/config folders, not a sandbox.

How it was checked: the install layout, the no-PATH-fallback rule, the no-global-install grep, the Antigravity
provisioning and the per-provider `npm_package` values were read directly. The environment-variable table, the
`AGENTMUX_ISOLATED_AUTH` behaviour and the seeding removal come from a read-only code search and were not re-read line
by line; confirm those citations when implementing.

### 2.1 What is true

| Claim | Evidence |
|---|---|
| AgentMux installs a pinned copy of each npm-managed CLI into its own folder. | `crates/srv/src/backend/cli_install.rs:62-77`: `<shared_dir>/cli/<provider>/<pinned_version>/`, `shared_dir` = `~/.agentmux/shared` (`crates/common/src/data_paths.rs:216`). Installed with `npm install --prefix <dir> <pkg>@<pin>` (`cli_install.rs:406-456`): no `-g`. |
| It runs that copy, never a system one, for those providers. | `crates/srv/src/server/cli_handlers.rs:29` ("Never falls back to system PATH for npm-backed providers"); the PATH lookup at `cli_handlers.rs:91-92` runs only when `npm_package` is empty. Same order in `agent.open` (`server/app_api/agent_open.rs:479-545`). |
| It does not touch the system's own installs. | A grep of `crates/` finds no `npm install -g` / `--global`, no PATH edit, no registry write for CLIs. |
| Each agent's login and config live in AgentMux's folders, not the system's. | `auth_config_dir_env_var` per provider points at `~/.agentmux/shared/providers/<name>/` (`crates/cef/src/commands/providers.rs:703-711`, set at `agent_open.rs:600-626`); per-account dirs under `shared/identities/<id>/<provider>` override (`identity_auth_dirs.rs:306-320`). Variables: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, `COPILOT_HOME`, `CLINE_DIR`, `OPENCLAW_HOME`, `DSH_HOME`, `QWEN_HOME`, `GROK_HOME`, `PI_CODING_AGENT_DIR`, `KIMI_SHARE_DIR`, `MUXCODE_CONFIG_DIR`, `GOOSE_PATH_ROOT`, `CRUSH_GLOBAL_DATA`, `XDG_DATA_HOME` (kilo, opencode), `XDG_CONFIG_HOME` (muse). |
| The system's credentials are not copied in. | `seed_provider_auth_from_global` was removed because it copied `~/.claude/.credentials.json` (`providers.rs:138-158`). |

### 2.2 Caveats the wording must respect

1. **Six providers are not managed.** `aider`, `antigravity`, `goose`, `grok`, `kimi`, `muse` have an empty `npm_package`
   (`crates/srv/src/backend/providers/<name>.rs`). AgentMux does not install them: its installer is npm-only
   (`install_handlers.rs` header) and none of these is on npm; each ships its own installer (section 4.3). It finds them
   on `PATH` or in the vendor installer's own folder and otherwise shows the install command
   (`cli_handlers.rs:106-124`). So for these the user's own install is what runs. Antigravity also reads no config-dir variable (`antigravity.rs:26-28`: it "signs in
   once per machine"), so it is not config-isolated either; Aider has no variable at all.
2. **Not a sandbox.** Agents still run on the user's machine with the user's `HOME` and `PATH`; `HOME` is not overridden
   (`crates/srv/src/backend/pane_env.rs:755-764` keeps `HOME` and `PATH`). `RuntimeBadge.tsx:35` already says host agents
   run "directly on your machine with full system access". The note must not suggest otherwise. For Claude, a
   placeholder `CLAUDE.md` is seeded because `CLAUDE_CONFIG_DIR` alone leaked the user-level `~/.claude/CLAUDE.md`
   (`providers.rs:609-637`, `docs/reports/REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §0). Whether other
   providers' instruction files (`AGENTS.md`, `GEMINI.md`) leak is untested.
3. **Node and npm come from the system.** The managed install needs a system Node and npm on `PATH`
   (`cli_handlers.rs:137-157`); the CLIs themselves are AgentMux's.
4. **"Shared" is the default.** The managed folder and the auth folders are shared by every AgentMux instance on the
   machine unless `AGENTMUX_ISOLATED_AUTH=1` (`data_paths.rs:719-780`). "Isolated" therefore means isolated from the
   system's `~/.claude`, `~/.codex` etc., not per build or channel.
5. **npm runs with the user's normal npm config** (for example `~/.npmrc`). Not checked whether AgentMux overrides it.
6. **Containers** install their own CLI into a per-agent volume (`container_cli.rs:119,189`); out of scope for the note.

## 3. Existing text, and what is stale

`agentmux-docs/src/content/docs/first-agent.md` already has a section "You don't need to preinstall the agent CLIs"
(lines 32-38) and `internals/provider-cli-integration.md:42-51` says the same. Both are **out of date** against the code:

| Where | Says | Code says |
|---|---|---|
| `first-agent.md` section "You don't need to preinstall…" | CLIs live in `~/.agentmux/instances/v<version>/cli/<provider>/`, "installed again for the new version the first time you use it". | `~/.agentmux/shared/cli/<provider>/<pinned CLI version>/`, shared by all instances; the old per-version folder is only a read-only fallback (`cli_install.rs:81-88,153-168`). A CLI is reinstalled when its pinned version changes, not when AgentMux updates. |
| `first-agent.md` provider table, Antigravity row | npm `@google/antigravity-cli` | Not on npm; `agy` is a native binary from Google's installer (`antigravity.rs:31-38`). |
| `first-agent.md` line "AgentMux installs every npm-based CLI" + "Kimi Code CLI is the exception" | Kimi is the only exception. | Six exceptions (above). |

No user-facing text in the app claims isolation. `README.md:72` says each *instance* is "fully isolated", which is about
instances, not CLIs, and must not be confused with this.

## 4. Proposed change

### 4.1 App: `frontend/app/view/agent/components/AgentPicker.tsx`

The "New Agent" tier already has a one-line hint under its header (lines 1135-1138,
`<p class="agent-picker-templates-hint" data-testid="agent-templates-hint">`). Add a **second** `<p>` with the same class
directly after it (before the grid at line 1139):

> AgentMux installs and runs its own copy of each agent CLI, kept apart from any you installed yourself, and keeps each
> agent's login and settings in its own folders. The exceptions use the copy on your PATH, so install them yourself
> first: Antigravity, Aider, goose, Grok Build, Muse Code, Kimi Code CLI. This isolates the tools, not what an agent can
> do on your computer.

(As built: the names are the catalog's providers with no npm package, each a link to its `docsUrl`, so the list follows
the catalog.)

- Each exception name is a link (`<a>` opening in the user's browser the way other external links in the app do) to the
  vendor's install page or script, so a user can install it and have it land on their PATH. The link targets and the
  exception list come from the provider catalog, not hand-written: `frontend/app/view/agent/providers/catalog.ts`
  already holds, per provider, `npmPackage` (empty for exactly these six) and `windowsInstallCommand` /
  `unixInstallCommand`, the hints the app already shows when a launch finds the CLI missing. Render the note for every
  catalog provider whose `npmPackage` is empty, so it cannot drift from the backend (this resolves open question 1).
- `data-testid="agent-templates-managed-note"`. Reuse the hint class (`styles/_picker.scss:159-164`); no new scss.
- Do **not** change the existing hint's wording: `AgentPicker.test.tsx:299-314` asserts it ("model it uses next", no
  `/model[a-z]/i`).
- Strings in this file are inline JSX literals (no i18n); a line break right after `</strong>` drops the space.
- Test: new `it(...)` beside line 299 asserting the note renders under the "New Agent" header, and that each catalog
  provider with an empty `npmPackage` appears in it as a link with an `href` (so adding a seventh PATH-only provider
  without a link fails the test).

### 4.2 Docs: `agentmux-docs/src/content/docs/first-agent.md` ("First Agent Setup")

- Add the same note (docs wording may be a little longer) at the top of the section "You don't need to preinstall the
  agent CLIs" (line 32), and **correct the stale facts in that section and the table** per section 3 (path layout,
  when a CLI is reinstalled, the six exceptions, Antigravity not on npm). One change, since a note beside a wrong path is
  worse than no note.
- Add a subsection, "Harnesses AgentMux doesn't install", with the table in section 4.3: for each of the six, the vendor's
  own installer and where it puts the CLI, and say plainly that these are the vendors' **full installers, outside
  AgentMux's isolation**: they install into the user's account and put the CLI on the user's PATH, and some update
  themselves. Say that AgentMux then runs whatever copy is on the PATH (or in the installer's own folder), so its version
  is the vendor's, not one AgentMux pinned.
- Check `internals/provider-cli-integration.md:42-51` for the same stale path and fix it in the same PR.
- The docs repo needs a changeset (`scripts/changeset.sh patch "<summary>"`), a PR title starting `<agent>@<host>: `,
  and `npm run build` locally; merging to `main` deploys to production. Cross-check content against the app source (the
  `src/agentmux` submodule). Do not touch generated API pages.
- The `agentmux-docs` clone must be brought up to `origin/main` before the submodule is updated (workspace rule after the
  2026-10-04 history rewrite).

### 4.3 The installers (source for the links and the docs table)

Install commands are the ones already in `frontend/app/view/agent/providers/catalog.ts` (the hints the app shows when a
launch finds the CLI missing); "lands in" is `known_install_paths` in `crates/srv/src/backend/providers.rs:412-500`.

| Harness | Windows | macOS / Linux | Lands in | Notes (from the code) |
|---|---|---|---|---|
| Antigravity (`agy`) | `irm https://antigravity.google/cli/install.ps1 \| iex` | `curl -fsSL https://antigravity.google/cli/install.sh \| bash` | `%LOCALAPPDATA%\agy\bin\agy.exe`; `~/.local/bin/agy` | Native binary; the installer adds its folder to the user's PATH and keeps the CLI up to date itself (`providers.rs:412-418`, `antigravity.rs:31-38`). Signs in once per machine. |
| Aider | `irm https://aider.chat/install.ps1 \| iex` | `curl -LsSf https://aider.chat/install.sh \| sh` | uv's tool bin dir, `~/.local/bin` | uv-based script (Python). |
| Goose | `irm https://github.com/aaif-goose/goose/releases/download/stable/download_cli.ps1 \| iex` | `curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh \| CONFIGURE=false bash` | `~/.local/bin` (Windows: also `~/goose` under Git Bash) | GitHub release binary. |
| Grok Build | `irm https://x.ai/cli/install.ps1 \| iex` | `curl -fsSL https://x.ai/cli/install.sh \| bash` | `~/.grok/bin` | xAI's installer. Also winget `xAI.GrokBuild` (harness table in `SPEC_NINE_NEW_HARNESSES_2026_10_14.md`). |
| Muse Code | `irm https://dev.meta.ai/install.ps1 \| iex` | `curl -fsSL https://dev.meta.ai/install.sh \| sh` | `%LOCALAPPDATA%\Programs\muse` (or `MUSE_INSTALL_DIR`); `~/.local/bin/muse` | Meta's launcher downloads a native binary (~450 MB) and updates itself, about hourly (`providers.rs:455-470`). |
| Kimi Code | `pip install kimi-cli` | `pip install kimi-cli` | the user's Python environment | A Python package, not a full installer; where it lands was not checked. |

Notes for whoever implements it:
- The scripts above are what the CLI vendors publish; the app links to them (or to the vendor's project page) and shows
  the command. A script URL is not a human-friendly page. Pick one rule, vendor project page where the catalog has one
  and the script otherwise, and verify every link resolves before shipping (open question 4). Not verified: whether each
  installer edits the user's PATH (only Antigravity's does, per the code comment); say "puts the CLI in <folder>" for the
  others.
- Do not paste the commands into the in-app note; the app already shows them at the point a launch fails (error
  AMX-CLI-004). The note's job is to say, up front, that these six need the user's own install.

### 4.4 "Click to install" and the Toolchain pane for the six (added 2026-10-18, operator's follow-up)

> [!NOTE]
> Superseded for detection and the ribbon by `SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md` §1 (one lookup for picker
> and Toolchain; the green ribbon for every provider whose CLI is not found, the six included) and §2.3 (the install
> step inline under the card instead of a modal). Step B below (a one-click run of vendor scripts) still stands as a
> separate, later spec.

The operator asked: use the same "Click to install" for these, and detect them in the Toolchain pane.

**What the UI does today (read, not changed):**
- **Picker cards never show the ribbon for the six.** `AgentCard.tsx:188` shows "Click to install" when
  `installed === false`, but `AgentPicker.tsx` `checkInstalled` (~line 371) returns early for any provider whose
  `npmPackage` is empty and leaves the state `undefined` ("never show the ribbon", comment at ~line 352). So a user picks
  Goose, and the first sign it is missing is a launch error (`CliMissingOnPath`, AMX-CLI-004, with the install hint).
- **The Toolchain pane already has a row for every provider**, the six included (`toolchain-view.tsx:132-139` builds one per
  `getProviderList()` entry) and probes each with the same `ResolveCliCommand` the launch uses (`:190-204`), which falls
  back to the system PATH for a provider with no npm package (`cli_handlers.rs:91-92`) and consults the vendor installer's
  own folder for the argv, terminal and Muse providers (`cli_handlers.rs:848-873`). So detection mostly exists. What is
  missing there: the row shows "Not found" with a button to the provider's docs, **no install command** (the provider
  rows set `installUrl` only, not `installCommand`), and nothing says the copy found is the user's own. `managed`
  (`toolchain-view.tsx:375`) marks AgentMux's copy; a PATH copy gets no label.
- I did not read the rest of `ResolveCli` (the branch for providers that are neither argv, terminal nor Muse) to confirm
  the vendor folder is consulted for all six; confirm for Antigravity, Goose and Grok when implementing.
- The in-app installer for system tools (`AgentPrereqModal.tsx:38`, `SystemToolInstallInline`) covers only git, node, npm
  and python. The agent-CLI installer (`install.start`) is npm-only.

**Proposal, in two steps so the small part is not held up by the large one:**

*Step A (small, same PRs as the note):*
- Toolchain: for the six, set the row's `installCommand` from the catalog (shown with the existing copy button) and add a
  muted pill, "your install", when `source === "system_path"`, beside the existing "managed" pill. Keep "Not found" +
  the link button.
- Picker: no ribbon yet, but the note (4.1) links each of the six.

*Step B (larger, its own spec/PR):* the real "Click to install" for the six.
- Detection: `checkInstalled` asks the same resolver for the six (found on PATH or in the vendor folder) and sets
  `installed` true/false, so the existing ribbon appears. Cheap.
- Install action: clicking a ribbon on one of the six opens an install modal that **shows the exact command, asks the user
  to press Run**, streams the output (the `install_chunk` events the npm path already uses) and re-detects through
  `known_install_paths` when it ends (the running srv's PATH is stale until restart, which is why those paths exist).
- This needs a **new backend path**: `install.start` runs `npm install` only. Running a vendor script (`curl … | bash`,
  `irm … | iex`) is arbitrary remote code execution, so the RPC must take a provider id and run only the command from a
  catalog **compiled into srv**, never a command string sent by the frontend or an agent, and it must be refused for any
  caller that is not the user's own window (agents must not be able to start it). Decide the confirmation wording, where
  Windows runs `irm | iex` (PowerShell, execution policy), what a failed script leaves behind, and that these installers
  put the CLI outside AgentMux's isolation (so the modal says so).
- Recommendation: do Step A now; write Step B as its own spec with the security design before any code.

### 4.5 Order

Two PRs, one per repo (Step A included in the app PR), either order; the docs PR should not claim anything the app PR's
note does not. Both are small. The app PR needs a changeset; the version is bumped by the release PR, not this one.
Step B is a separate, later PR.

## 5. Open questions

1. ~~Hard-code the six exceptions or derive them?~~ Resolved: derive from the catalog's `npmPackage` (section 4.1).
2. Is "isolated from the system" the right word for the **docs**, given caveat 2.2.2 (not a sandbox)? The proposed copy
   says "kept apart" and ends with the sandbox sentence; say if you want it shorter.
3. Antigravity is not config-isolated either (it signs in once per machine). Should the note say so, or is "use the copy
   on your PATH" enough? Recommend enough; the docs table already says it.
4. Link targets: vendor project page or installer script for each of the six? The script URLs are verified only as
   strings in the catalog, not fetched.

## 6. Not in scope

Sandboxing agents, overriding `HOME`, an "use my own CLI" setting (none exists today; `ProviderSettings.cli_path` in
`crates/cef/src/commands/providers.rs:35` was not traced into the spawn path), container agents, and the stuck
shared-versus-per-channel auth question (`AGENTMUX_ISOLATED_AUTH`).
