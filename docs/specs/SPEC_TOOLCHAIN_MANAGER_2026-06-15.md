# SPEC: Toolchain Manager + GUI-launch PATH enrichment

- **Date:** 2026-06-15
- **Status:** Draft
- **Author:** AgentO
- **Related:** `SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md` (system-prereq probe + modal), `SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14.md`

---

## 0. TL;DR

Two coupled deliverables:

1. **P0 — Fix "NPM failed" at the root.** When AgentMux is launched from Finder/Dock/DMG as a `.app`, `agentmux-srv` inherits launchd's stripped PATH (`/usr/bin:/bin:/usr/sbin:/sbin`). nvm/Homebrew-installed `node`/`npm` are **not** on it, so `install.start`'s `Command::new("npm")` fails with `npm: command not found`. Enrich the process PATH from the user's **login shell** (+ well-known toolchain dirs) so GUI launches can find the toolchain. This fixes both CLI **install** and CLI **execution**.

2. **Toolchain Manager modal** — a new hamburger (≡) menu item → a window-scoped modal that gives users **visibility and control** over the toolchain AgentMux uses: detected versions + paths for `node`, `npm`, `git`, `docker`, and every provider CLI, with **install / repair in place** and the **resolved PATH** AgentMux is actually using (so PATH problems are diagnosable, not mysterious).

The PATH fix removes the failure; the Toolchain modal makes the toolchain *legible and fixable* by the user instead of a black box.

---

## 1. Problem

### 1.1 The acute bug (reproduced)
- The running 0.44.1 GUI app's srv has `PATH=/usr/bin:/bin:/usr/sbin:/sbin` (read live off the process).
- The user's `npm` is nvm/Homebrew-managed (`/opt/homebrew/opt/node@20/bin/npm`, `~/.nvm/...`), none on that PATH.
- Repro: `env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin npm install … → sh: npm: command not found`.
- `install_handlers.rs` spawns `Command::new("npm")` relying on PATH (`agentmux-srv/src/server/install_handlers.rs:446`). No PATH enrichment exists anywhere in the repo (grep for login-shell capture returns nothing).
- **Blast radius:** the majority of devs install Node via nvm/Homebrew → most macOS (and many Linux) alpha users hit "NPM failed" the moment they run the packaged app normally. This is the central "do users have the right prereqs" risk for the 0.45.0 alpha.

### 1.2 The deeper gap
There is **no surface** where a user can see what AgentMux detects, what PATH it's using, what versions are installed, or fix a missing/broken tool. Prereq feedback today is reactive (a pre-launch `AgentPrereqModal`, a Node banner) and per-provider. Users have **no power** to inspect or manage the toolchain centrally.

### 1.3 Secondary oddity — RESOLVED (not a bug)
v0.44.1's `@anthropic-ai/claude-code@2.1.173` maps `bin.claude → bin/claude.exe`. Investigated: `bin/claude.exe` is a **Mach-O 64-bit arm64** executable (not a Windows PE), runs correctly (`claude.exe --version → 2.1.173 (Claude Code)`), and the package ships per-platform `optionalDependencies` (`@anthropic-ai/claude-code-darwin-arm64` is installed as a sibling; the `install.cjs` postinstall copies the platform-correct binary to the fixed name `bin/claude.exe`). The `.exe` suffix is a cosmetic fixed filename, **not** corruption or a cross-platform mistake. **The install is healthy** — "NPM failed" was purely the PATH issue (§1.1). Consequence for the modal: "Repair" does **not** need to special-case this; a normal reinstall is fine.

---

## 2. Goals / Non-goals

**Goals**
- GUI-launched AgentMux resolves the same toolchain the user's terminal does (P0).
- A single Toolchain modal: detect + show versions/paths for node, npm, git, docker, and all provider CLIs.
- Install/repair provider CLIs **in place** from the modal (reuse `install.start`).
- Surface the **effective PATH** AgentMux is using + its source (login-shell / enriched / fallback) for self-diagnosis.
- Guided install for non-npm system tools (node/git/docker) — links + copyable commands, and one-click where a trusted package manager is detected (e.g. `brew`).

**Non-goals (this iteration)**
- Auto-installing Docker or Node silently (we link/guide; we do not background-install heavyweight runtimes without consent).
- Managing multiple Node versions / acting as a version manager.
- Windows-specific deep PATH heuristics beyond the existing `where`-based probes (Windows GUI apps inherit a usable PATH; the launchd-stripping problem is macOS/Linux-shaped).

---

## 3. P0 — GUI-launch PATH enrichment

### 3.1 Where
Enrich **once, in the host before it spawns the srv**, so the srv and every CLI it spawns inherit the corrected PATH. Anchor: `agentmux-cef/src/sidecar.rs:197-226` (the `Command::new(&backend_path)…envs(...)` site). A fallback enrichment in `agentmux-srv/src/main.rs` (~`fn main` line 229) guards the case where srv is launched directly (`task dev` on macOS/Linux invokes the host which spawns srv; standalone srv launches are dev-only).

Decision: implement the resolver in `agentmux-common` (shared) as `resolve_login_path()` and call it from the host's sidecar spawn. Single source of truth, testable.

### 3.2 How — `resolve_login_path()`
1. **Detect a stripped PATH.** If the current PATH ⊆ the known launchd-default set (`/usr/bin:/bin:/usr/sbin:/sbin`) OR is missing a node/npm/git, attempt enrichment. (Always attempt on macOS GUI; cheap.)
2. **Capture the login-shell PATH.** Run `$SHELL -lic 'printf "%s" "$PATH"'` (fallback `/bin/zsh` on macOS, `/bin/bash` on Linux) with:
   - a hard **timeout** (≤2s) — never block startup on a slow rc file;
   - `stdin` null, output captured;
   - failure → skip to step 3 only.
   Verified working: `$SHELL -lic 'echo $PATH'` returns the full nvm/Homebrew PATH on this machine.
3. **Union with well-known toolchain dirs** (existence-checked, de-duplicated, login-shell entries first):
   - **All Unix:** `~/.nvm/versions/node/<current|default>/bin`, `/opt/homebrew/bin`, `/opt/homebrew/sbin`, `/usr/local/bin`, `~/.local/bin`, `~/.cargo/bin`, `/opt/local/bin`. `<current>` for nvm resolved via `~/.nvm/alias/default` then highest `vNN`.
   - **Linux also:** `/snap/bin`, `/var/lib/flatpak/exports/bin`, `~/.local/share/flatpak/exports/bin` (resolves Q4 — Snap/Flatpak-packaged git/node/docker). All existence-checked, so absent dirs cost nothing.
4. **Merge** the result over the inherited PATH (enriched entries take precedence; never *drop* inherited entries).
5. **Record** the final PATH + its source (`login-shell` | `fallback-dirs` | `inherited`) so the Toolchain modal and logs can show it.

### 3.3 Security / correctness
- Running `$SHELL -lic` sources the user's own rc files — same trust boundary as their terminal; no privilege escalation. Document the timeout + that we only read `$PATH` (no other env exfil).
- Pure-additive to PATH: we never remove the system dirs, so we can't break system tool resolution.
- Windows: no-op (return inherited PATH). Gate the shell invocation behind `#[cfg(unix)]`.
- Idempotent: safe to call once at host startup; result cached.

### 3.4 Tests
- `resolve_login_path()` unit tests with a fake `$SHELL` script emitting a known PATH (timeout path, non-zero-exit path, empty-output path → fallback).
- Existence-filtering + de-dup + precedence ordering.
- Windows cfg returns inherited unchanged.

---

## 4. Toolchain Manager — UX

### 4.1 Entry point
Add to `frontend/app/window/hamburger-menu.tsx` `menuItems()` (~line 107, before the Settings group):
```ts
{ label: "Toolchain", icon: "wrench", onClick: () => openModal(ToolchainModal) },
```
`openModal` already imported from `@/app/store/modalmodel`. `ToolchainModal` receives the auto-injected `{ close }` prop, wrapped in `<Modal scope="window">` (`frontend/app/element/modal.tsx`).

### 4.2 Layout
A window-scoped modal, three sections:

1. **Environment** (top, collapsible)
   - Effective PATH AgentMux is using + **source badge** (`login shell` / `fallback dirs` / `inherited`).
   - OS + arch, AgentMux version + channel, instances dir.
   - "Copy diagnostics" button (PATH + all detected versions) for bug reports.

2. **Core tools** — fixed rows: `node`, `npm`, `git`, `docker`.
   - Each row: icon, name, **detected version** (or "Not found"), resolved path, status pill (✓ found / ⚠ missing / ⚠ outdated-vs-min).
   - Action: **external tools** (node/git/docker) → "Install ↗" (platform install URL) + a copyable command, **always available** (v1 baseline). One-click "Install with Homebrew" is a **P3 add-on** (resolves Q3): shown only when `brew` is detected, requires an **explicit confirm**, runs `brew install <formula>` through the existing streamed-install channel with live output, and is **never** auto-run. Link+command remain the always-present fallback so the modal is useful even without brew.
   - `node` row shows the Node **minimum** (18+) and flags outdated **warn-only** — a ⚠ pill + "Node 18+ recommended", never a launch block (resolves Q2). Rationale: exact-path version detection is fuzzy, claude's native binary needs no Node, and blocking is hostile; the existing Node banner stays warn-only too.

3. **Agent CLIs** — iterate `Object.values(PROVIDERS)` from `frontend/app/view/agent/providers/index.ts`.
   - Each row: provider icon + displayName, detected version + source (`local_install` versioned dir / `system_path`), status pill.
   - Action: **managed (npm) providers** → **Install / Repair** button in place (reuses the `install.start` streamed flow, same as `AgentInstallModal`); **non-npm** (kimi/pip, claude/native) → guided install (docs link + command).
   - Show declared `systemPrereqs` per provider (e.g. claude→git) with their own found/missing state, so "claude needs git and git is missing" is visible here.

### 4.3 Install / repair in place
- Reuse `RpcApi.InstallStartCommand({ providerId, cliCommand, npmPackage, pinnedVersion })` → `{ sessionId }`, subscribe to `install_chunk` WPS events scoped `install:<sessionId>` (exact pattern in `AgentInstallModal.tsx:121-158`). Render live output inline in the row (expandable) or a sub-panel.
- "Repair" = re-run `install.start` with the pinned version (idempotent reinstall into the versioned dir).
- After completion, re-probe that row.

---

## 5. Detection — RPC surface

### 5.1 Reuse
- Provider CLI version/path: `RpcApi.ResolveCliCommand` (`cli_handlers.rs:14`) already returns `{ cli_path, version, source }` and is the exact call `launch-flow.ts:75` uses for docker. Use it for each provider **and** for `docker` (`provider_id:"docker", cli_command:"docker", npm_package:""`).
- PATH presence/path for arbitrary tools: `RpcApi.ResolvePrereqsCommand({ tools })` → `{ tool, found, path }[]` (`install_handlers.rs:101`).
- Node/npm: host API `getApi().checkNodejsAvailable()` → `{ available, version, npm_available, npm_version, path }` (`cef-api.ts:595`).

### 5.2 New — `toolchain.status` (batch)
Add one srv RPC that returns the whole picture in a single round-trip (avoids N sequential probes + gives the enriched PATH the srv is using):
```ts
ToolchainStatusCommand(client): Promise<{
  path: string;                 // effective PATH the srv is using
  pathSource: "login-shell" | "fallback-dirs" | "inherited";
  os: string; arch: string;
  tools: Array<{
    id: string;                 // "node" | "npm" | "git" | "docker" | provider id
    kind: "core" | "provider";
    found: boolean;
    version: string | null;     // via get_cli_version (cli_handlers.rs:514) — runs `<tool> --version`, 5s timeout
    path: string | null;        // which/where, or versioned install dir
    source: "system_path" | "local_install" | null;
    managed: boolean;           // true → installable via install.start (npm providers)
    minVersion?: string;        // e.g. node "18"
  }>;
}>
```
Backend: assemble from `resolve_tool_path` + `get_cli_version` (both already in `cli_handlers.rs`/`install_handlers.rs`) + the enriched-PATH record from §3.5. `git`/`docker`/`node`/`npm` probed by name; providers probed via the versioned-dir-then-PATH logic already in `resolve.cli`. Reuse `is_safe_cli_command` validation.

### 5.3 Toolchain catalog
Introduce a small **core-tools catalog** (node, npm, git, docker; each with id, label, icon, minVersion?, install URLs per platform, optional `brewFormula`) alongside the existing provider list. Put it next to `providers/index.ts` (e.g. `toolchain-catalog.ts`) so the modal has one place to add "anything we need" later (ripgrep, uv/python for kimi, etc.). This also lets us fold in the deferred kimi-Python prereq from the prereq audit.

---

## 6. Rollout

- **P0 (ship first, standalone):** §3 PATH enrichment in `agentmux-common` + host sidecar spawn. Independently valuable; unblocks installs immediately. Its own AgentO PR + reagent + changeset (`fix`).
- **P1:** `toolchain.status` RPC + core-tools catalog + read-only Toolchain modal (Environment + Core + Agent CLI sections, versions/paths/PATH-source, links only).
- **P2:** Install/Repair in place for npm providers (wire `install.start` into rows).
- **P3:** One-click `brew install` for core tools where `brew` is detected; fold in kimi Python prereq + Node min-version flagging.

Each phase is a separate PR through the AgentO → reagent → squash flow.

---

## 7. Touch points (files)

- `agentmux-common/src/` — new `resolve_login_path()` (+ tests).
- `agentmux-cef/src/sidecar.rs:197` — call enricher before spawning srv; record PATH source.
- `agentmux-srv/src/main.rs:229` — fallback enrichment for direct-launch.
- `agentmux-srv/src/server/install_handlers.rs` / `cli_handlers.rs` — `toolchain.status` handler (reuse `resolve_tool_path`, `get_cli_version`, `is_safe_cli_command`).
- `frontend/app/store/rpc-api.ts` — `ToolchainStatusCommand`.
- `frontend/app/window/hamburger-menu.tsx:107` — menu item.
- `frontend/app/view/.../ToolchainModal.tsx` — new modal (reuse `Modal`, `install_chunk` pattern from `AgentInstallModal.tsx`).
- `frontend/app/view/agent/providers/toolchain-catalog.ts` — new core-tools catalog.

---

## 8. Tests

- `resolve_login_path()` unit tests (§3.4).
- `toolchain.status` handler: tool found/missing, version parse, provider local-install vs system-path, PATH-source field, `is_safe_cli_command` rejection.
- Frontend: modal renders rows from a mocked `toolchain.status`; Install button calls `install.start` and re-probes on `done`.

---

## 9. Resolved decisions

All four open questions are now resolved (decisions baked into §1.3, §3.2, §4.2 above):

1. **claude.exe on macOS — RESOLVED, not a bug.** `bin/claude.exe` is a Mach-O arm64 binary that runs correctly (`2.1.173`); the package uses per-platform `optionalDependencies` and `install.cjs` writes the platform binary to a fixed `.exe` filename. The v0.44.1 install is healthy. **"Repair" needs no special-casing** — a normal reinstall suffices (see §1.3). The only failure was the PATH issue.
2. **Node min-version — RESOLVED: warn-only.** Show a ⚠ "Node 18+ recommended" pill in the modal + keep the existing banner warn-only. Never block launch (§4.2). Fuzzy version detection + claude's no-Node native binary make blocking inappropriate.
3. **One-click brew — RESOLVED: P3 add-on, link+command is v1 baseline.** Always show install URL + copyable command. Add one-click `brew install` only in P3, only when `brew` is detected, behind explicit confirm, via the streamed-install channel, never auto-run (§4.2).
4. **Linux PATH — RESOLVED: include Snap + Flatpak.** Add `/snap/bin`, `/var/lib/flatpak/exports/bin`, `~/.local/share/flatpak/exports/bin` to the existence-checked dir union (§3.2 step 3).

No remaining blockers — the spec is ready to implement starting at P0.
