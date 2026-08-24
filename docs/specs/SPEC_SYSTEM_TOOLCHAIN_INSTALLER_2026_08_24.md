# SPEC: One-click system-toolchain installer (git, Node/npm, and friends) across Windows/macOS/Linux

**Date:** 2026-08-24
**Status:** implemented 2026-08-24 — §3.1-§3.5 for git/Node/npm/Python
across Windows/macOS/Linux (Phases 1+2 of §6). Backend: 6 new Rust unit
tests, full 2774-test suite green. Frontend: `npx tsc --noEmit` clean,
full 3092-test vitest suite green (5 new). §6.3's Phase 3 (bootstrapping a
MISSING package manager itself) and §6.4's NodeSource-vs-bare-package
decision remain explicitly **not implemented** — flagged, not defaulted.
The §3.1 Windows UAC-prompt-origin question is **not yet verified on real
hardware** — the winget command construction is implemented and unit-
tested, but whether the OS's own elevation prompt reliably attaches when
spawned from this app's non-console process is an open manual-verification
item (see §5's manual test plan), same honesty posture as this repo's
other "implemented, live-environment check still pending" PRs. `uv` and
Docker are intentionally excluded from the executable-install catalog
(script-based install / interactive GUI installer — different risk
profile, not requested); their existing link+copy-command rows are
unchanged.
**Related:** `docs/specs/SPEC_TOOLCHAIN_MANAGER_2026-06-15.md` (shipped
P0-P1: PATH enrichment + read-only Toolchain modal; **this spec implements
its deferred P3** — "one-click brew install… P3 add-on, not yet shipped" —
generalized to Windows/Linux, not brew-only), `docs/specs/SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md`
(the reactive `AgentPrereqModal` blocker this spec also wires into),
`docs/specs/SPEC_AGENT_INSTALL_STAGE_2026_05_17.md` (the `install.start`
streamed-install machinery this spec reuses),
`docs/specs/SPEC_TOOLCHAIN_MANAGER_EXTERNAL_WIDGETS_2026_06_22.md`
(extends the same catalog to Python/uv + widget tools — same pattern,
this spec's design should stay compatible with it).

---

## 1. Report

Users hit a hard wall the moment an agent's shell needs `git` or `npm` and
either isn't installed or isn't on PATH — this is a real, repeated
onboarding blocker, not a hypothetical. AgentMux already has **two**
places that know about this class of problem, and both stop at
"tell the user, give them a link":

1. **Reactive (`AgentPrereqModal.tsx`)** — fires at agent-launch time when
   a provider's declared `systemPrereqs` (today: just `git`, on
   claude/openclaw — `frontend/app/view/agent/providers/catalog.ts:106,325`)
   aren't on PATH. Shows an install-URL link per missing tool and a
   "Launch anyway" override. **Never installs anything.**
2. **Proactive (`toolchain-view.tsx` / `toolchain-catalog.ts`)** — the
   hamburger-menu "Toolchain" modal lists `node`/`npm`/`git`/`docker`/
   `python`/`uv` with detected version/path and, per
   `toolchain-view.tsx:286-299`, a copyable `installCommand` string
   (`code>{row.installCommand}</code>` + a copy button) and an "Install ↗"
   button that **only opens the tool's official download page**
   (`open(row.installUrl)` — no command execution anywhere in this file
   or `install_handlers.rs`). `CoreTool.brewFormula` exists on the catalog
   type (`toolchain-catalog.ts:58`) but is **never read** by any spawn
   code — it's inert data, a placeholder for the feature this spec
   builds. `SPEC_TOOLCHAIN_MANAGER_2026-06-15.md` §4.2/§6 designed this
   exact gap as "P3 — one-click `brew install`… not yet shipped" and
   scoped it to macOS/brew only; this spec is that P3, generalized to all
   three platforms.

**What's missing, concretely (verified against the catalog, not
assumed):** `installCommand` has **no Windows entry at all** for
`node`/`npm`/`git`/`docker` (`toolchain-catalog.ts:93,104,120` — only
`macos`/`linux` keys) — a Windows user gets a bare download link and
nothing copyable, worse coverage than macOS/Linux today. There is **no**
Linux distro-family detection anywhere in the Rust backend (`grep` for
`/etc/os-release`, `dnf`, `pacman`, `zypper` across `agentmux-srv/src` and
`agentmux-common/src` returns nothing) — the one Linux command shown
(`sudo apt install -y ...`) is silently wrong on Fedora/Arch/openSUSE.
There is **no** privilege-elevation handling anywhere in the codebase
(no `sudo`, `pkexec`, or UAC-elevation code) — every install action this
app has ever executed (`install.start`'s `npm install`) runs unprivileged,
into an AgentMux-owned directory; a system package-manager install is a
fundamentally different, higher-trust class of action this codebase has
never done before.

## 2. Goals / Non-goals

**Goals**
- A real "Install" action (not just a link) for `git`/`node`/`npm` (and,
  opportunistically, the rest of `CORE_TOOLS` — `docker`, `python`, `uv`)
  on Windows, macOS, and Linux, reusing the existing streamed-install UI
  pattern (`install_chunk` WPS events, live output, same visual language
  as `AgentInstallModal`).
- Wired into **both** existing surfaces: the reactive `AgentPrereqModal`
  (the actual blocker moment) and the proactive Toolchain modal (today's
  copy-command fallback becomes a real button when a package manager is
  available; the fallback stays as-is when one isn't).
- Correct, distro-aware command construction on Linux (no more blanket
  `apt install` shown on non-Debian systems).
- Every executed command comes from a **fixed, backend-owned catalog** —
  the frontend can request "install git," never an arbitrary command
  string. Same allowlist discipline as `is_safe_cli_command`/
  `is_safe_provider_id` today, extended to a strictly bigger blast radius
  (elevated system commands, not an unprivileged install into
  `~/.agentmux/...`).
- Full transparency before anything elevated runs: show the exact command,
  require an explicit click, never silent/background.

**Non-goals (this iteration — see §6 for what's phased vs. flagged
[DECISION NEEDED])**
- Auto-bootstrapping a *missing package manager itself* (Homebrew on a
  mac that's never had it; a pre-App-Installer Windows without `winget`).
  Both are materially bigger trust actions than "run a formula install
  through a package manager the user already opted into" — flagged, not
  defaulted to yes (§6.3).
- A general-purpose "run any elevated command" primitive. This feature is
  a small, fixed, versioned catalog of (toolId → per-platform install
  command), not a shell.
- Version pinning / "install exactly Node 20.11" — package managers
  install whatever their repo currently has; that's an accepted, existing
  limitation of every `installCommand` shown today too.
- Docker Desktop installation (still `optional: true`, still link-only —
  Docker Desktop's own installer is a large, interactive GUI flow that
  doesn't fit a headless streamed-install session; unchanged from today).

## 3. Design

### 3.1 Per-platform package-manager strategy

**Windows — `winget`.** Bundled with Windows 11 and modern Windows 10
(App Installer, auto-updated via the Store) — no bootstrap needed for the
overwhelming majority of target machines. Command shape:
`winget install --id <PackageIdentifier> --silent --accept-package-agreements --accept-source-agreements`.
Elevation: winget handles its own UAC prompt per-package when a package's
installer requires it (Git-for-Windows's installer does; Node's MSI
does); AgentMux does **not** need to pre-elevate the `winget` process
itself for a per-user-scope install. **[DECISION NEEDED — research
spike, not assumed]:** does a UAC prompt raised by a child process spawned
from a non-console GUI app (`agentmux-srv`, itself spawned by
`agentmux-cef`/the launcher) reliably surface and attach to the correct
desktop session? This needs to be verified on a real Windows box before
Phase 2 (§6) ships — if it doesn't reliably attach, the fallback is
spawning through `ShellExecuteW` with `runas`, which is a different code
path than the plain `tokio::process::Command` this codebase uses
everywhere else. If `winget` itself is missing (rare, old Windows), no
auto-bootstrap — same non-goal posture as today; keep the existing
download-link fallback.

**macOS — `brew`.** Exactly the already-designed P3
(`SPEC_TOOLCHAIN_MANAGER_2026-06-15.md` §4.2): `brew install <formula>`,
never run as root (Homebrew refuses most operations under `sudo` by
design, so there is no elevation step to build — the *simplest*
platform here). Only reachable when `brew` is already detected on PATH.
If it's missing, fall through to the existing link+copy-command UI
unchanged (auto-bootstrapping Homebrew itself is §6.3, not this).

**Linux — detect the system package manager, no bootstrap ever needed.**
Every mainstream Linux distro ships with its own package manager
pre-installed as part of the base OS — unlike brew/winget, there is no
"package manager itself might be missing" case to design around. Detect
by checking for each manager's binary on PATH in priority order (first
match wins, since a system with e.g. both `apt` and a manually-installed
`brew` should prefer the native one): `apt-get` → `dnf` → `yum` → `pacman`
→ `zypper` → `apk`. Cross-check against `/etc/os-release`'s `ID`/
`ID_LIKE` fields where the mapping is ambiguous (e.g. distinguishing a
`dnf`-based immutable/Silverblue variant that may need `rpm-ostree`
instead — out of scope for v1, falls through to link+copy-command).
Elevation: **`pkexec <package-manager> install -y <package>`** —
`pkexec` triggers the desktop's own native polkit authentication dialog
(password or fingerprint, whatever the session's polkit agent provides)
without AgentMux ever rendering a password field itself. If no polkit
agent is running (minimal window managers, some server-oriented desktop
setups) `pkexec` fails fast with a clear error — falls through to the
existing copy-command-into-your-own-terminal UX, never a dead end.

**Package-name mapping caveat (real, not hypothetical) — Node on Debian/
Ubuntu.** The `apt` repositories' default `nodejs`/`npm` packages are
frequently far behind current LTS (a long-standing, well-known Debian/
Ubuntu packaging gap). A naive `sudo apt install -y nodejs npm` (today's
`toolchain-catalog.ts:93` literal string) reproduces that gap. The
per-platform catalog entry for `node` on `apt`-family systems should point
at the NodeSource setup script path instead (documented, official,
distro-recommended) rather than the bare distro package — this is a
**content decision for the catalog data**, not a code-architecture one;
flagged here so it isn't silently carried forward unfixed. `git` has no
equivalent gap (distro `git` packages are reasonably current everywhere
that matters).

### 3.2 Command catalog — the actual security boundary

New Rust-side data structure, `SystemInstallCatalog` (analogous to, and
living alongside, the existing frontend `CORE_TOOLS` catalog — but this
one is the **execution-authoritative** copy; the frontend catalog stays
display-only, exactly as today):

```rust
struct SystemInstallStep {
    program: String,        // e.g. "winget", "brew", "pkexec"
    args: Vec<String>,      // e.g. ["install", "--id", "Git.Git", "-e", "--silent"]
    needs_elevation: bool,  // informs the confirm-step copy shown to the user
}

// Keyed by (tool_id, platform, package_manager). Every command is a fixed
// argv Vec<String> — NEVER a shell string passed through `sh -c`/`cmd /c`,
// so there is no interpolation surface even though every entry in this
// table is itself hardcoded (defense in depth: a future catalog edit that
// accidentally introduces a format!()-built arg still can't smuggle shell
// metacharacters through a Vec<String> argv the way it could through a
// single command string).
```

The RPC surface takes **only** a `tool_id` (validated against a fixed
allowlist, same style as `is_safe_provider_id`) — never a package-manager
package name, never a raw command, from the frontend. The frontend's job
is to display the resolved command (fetched from the backend, which is
the one source of truth for what will actually run) for the consent step
in §3.3 — it never constructs or edits it.

New RPC: `toolchain.install_system_tool` (parallel to `install.start`,
same session/streaming/cancel shape from `InstallSessionRegistry`):
```ts
ToolchainInstallSystemToolCommand(client, { toolId: string }): Promise<{ sessionId: string }>
```
Backend resolves `(toolId, detected_platform, detected_package_manager)` →
the catalog's `SystemInstallStep`, spawns `Command::new(step.program).args(step.args)`
(plain piped stdio, matching `install.start`'s existing pattern — no PTY
needed for `winget`/`brew`/`pkexec`'s own non-interactive `-y`/`--silent`
flags), streams stdout+stderr line-by-line to `install_chunk` events
scoped `install:<sessionId>` — **identical wire format** to the existing
npm-install flow, so `AgentInstallModal`'s streaming-log rendering code
is reusable as-is for this new session kind, not a fork.

### 3.3 Consent — the non-negotiable step before anything elevated runs

Before `toolchain.install_system_tool` is ever called, the frontend shows
a confirm step (a small panel, not a native `confirm()`) displaying:
- The exact resolved command (fetched via a read-only
  `toolchain.resolve_install_command({ toolId })` preview call — same
  catalog lookup as the execute path, but zero side effects, so the user
  sees precisely what they're about to approve).
- Whether it needs elevation (`needs_elevation`), phrased plainly ("This
  will ask for your password" / "This will show a Windows permission
  prompt").
- An explicit **Install** button — no default-focus auto-trigger, no
  countdown, no "don't ask again" checkbox that could silently promote
  this to unattended in a future session.

This directly fulfills, rather than relaxes, the existing spec's stated
non-goal ("we do not background-install heavyweight runtimes without
consent," `SPEC_TOOLCHAIN_MANAGER_2026-06-15.md` §2) — it's the consent
mechanism made real, not a loosening of that posture.

### 3.4 Cancellation — narrower than the npm flow, on purpose

`install.start`'s existing cancel semantics (kill child, `rm_rf` the
partial install dir) are safe because an npm install only ever touches an
AgentMux-owned directory. A system package-manager transaction (`apt`/
`dpkg`, `winget`'s MSI/EXE installers, `brew`'s own transactional model)
can leave broken system state if killed mid-write. **Cancel is only
offered before the privileged command actually starts** (i.e., during the
consent step, or while `pkexec`/UAC's own auth prompt is still pending) —
once the underlying package manager has begun, the UI shows progress only,
no cancel button, mirroring how a real terminal `apt install` behaves
today (Ctrl-C mid-`dpkg`-unpack is already inadvisable outside this app
too — this isn't a new restriction, just not pretending otherwise).

### 3.5 Wiring into the two existing surfaces

- **`AgentPrereqModal.tsx`** — each `MissingPrereq` row gains an "Install"
  button (rendered only when `toolId` is in the backend's install catalog
  for the detected platform+package-manager; otherwise the existing
  link-only row is unchanged). On success, calls the same `onRefresh()`
  the modal already exposes — no new refresh mechanism.
- **`toolchain-view.tsx`** — the existing "Install ↗" button
  (`toolchain-view.tsx:297`, currently `open(row.installUrl)` only)
  becomes conditional: when the catalog resolves an installable command
  for this row+platform, it opens the §3.3 consent panel instead of the
  external URL; when it doesn't (no package manager detected, or the
  tool has no catalog entry — e.g. Docker, which stays link-only per §2's
  non-goals), it falls back to today's exact behavior. The copyable
  `installCommand` text stays visible either way — it's the correct
  fallback for a user who prefers running it themselves in their own
  terminal, or whose desktop has no polkit agent for `pkexec` to hand off
  to.
- **Redundant-entry note:** `node` and `npm` are two separate `CoreTool`
  rows that already both resolve to the same install target (npm ships
  bundled with Node — `toolchain-catalog.ts:93-105` show both entries
  pointing at `brewFormula: "node"` / `brew install node`). Wiring real
  execution behind two independent buttons risks a confusing double
  install-and-consent-prompt if a user clicks both in sequence. Recommend
  the `npm` row's "Install" action simply re-triggers the same session as
  `node`'s (or is visually merged into one action) — a UI-only decision,
  not a backend catalog change.

## 4. Security

- **Fixed catalog, not free-form execution.** Reiterating §3.2's core
  invariant since it's the entire security argument for this feature:
  the RPC boundary accepts a `tool_id` from a small, backend-owned
  enum-like allowlist — structurally identical to how `is_safe_cli_command`
  already gates `install.start`, just applied to a category of action
  (elevated system commands) that has a much higher cost if that
  discipline is ever skipped at a new call site.
- **No shell string interpolation, ever.** Every catalog entry is a
  `Vec<String>` argv passed straight to `Command::new(program).args(args)`
  — never `sh -c`/`cmd /c` with a formatted string. This is true even
  though every entry is hardcoded today (no user input reaches the argv
  at all in v1) — it's the same "don't build the injection surface even
  when nothing currently exploits it" posture as the rest of this
  codebase's command-spawning code.
- **Elevation is delegated to the OS's own consent UI** (`pkexec`'s
  polkit dialog, Windows' native UAC prompt, brew's no-root-needed model)
  — AgentMux never asks for or handles a sudo password itself, never
  stores one, never runs as an elevated process for longer than the one
  package-manager invocation.
- **Residual risk, stated plainly:** a `winget`/`brew`/`pkexec` package
  install can still run arbitrary code as an implicit consequence of what
  "installing a package" fundamentally means (pre/post-install scripts
  are a normal, expected part of every package manager on earth) — this
  is the same trust boundary a user already crosses running `brew install
  git` in their own terminal; this feature doesn't create a new kind of
  risk, it automates a command the user could already type themselves,
  with the exact command shown before it runs. Flagged for completeness,
  not as something this design can or should try to eliminate.

## 5. Test plan

- Rust: catalog resolution — `(tool_id, platform, package_manager)` →
  expected `SystemInstallStep` for every populated combination; unknown
  `tool_id` → `None`/error, never a fallback that guesses a command;
  Linux package-manager detection priority order given a fake PATH with
  0/1/2+ managers present; `/etc/os-release` parsing for the Node/apt
  NodeSource-vs-bare-package distinction (§3.1).
- Rust: `toolchain.install_system_tool` handler — spawns the resolved
  argv (assert on the `Command` built, not an actual execution, in unit
  tests — mirrors how `install.start`'s tests likely already avoid a real
  `npm` invocation); streams `install_chunk` events in the same shape as
  the existing npm flow; cancel is rejected once the child has started
  (§3.4), accepted before.
- Frontend: `AgentPrereqModal` renders an Install button only when the
  backend catalog resolves a command for the current platform; the
  consent panel shows the exact resolved command text (mocked RPC);
  `toolchain-view.tsx`'s row falls back to the existing link+copy-command
  behavior when no command resolves.
- **Manual, per-OS (this feature is fundamentally environment-dependent —
  unit tests can't cover "does `pkexec` actually work on a real desktop
  session," "does `winget` actually resolve `Git.Git`," etc.):**
  - [ ] Windows: install git via winget from a clean VM without git;
    confirm the UAC/consent flow's origin question from §3.1.
  - [ ] macOS with brew already present: install node via the Toolchain
    modal; confirm no sudo prompt (brew must not run as root).
  - [ ] Ubuntu/Debian, Fedora, Arch (three real or containerized distros
    minimum): confirm the correct package manager is detected on each and
    the resulting command is distro-correct, not a blanket `apt`.
  - [ ] A machine with no polkit agent running (or `winget`/`brew`
    genuinely absent): confirm graceful fallback to the existing
    link+copy-command UI, not an unhandled error.

## 6. Rollout & open decisions

1. **Phase 1 — Linux + macOS.** Both platforms have a package manager
   that's essentially guaranteed already present (Linux: always, it's
   part of the base OS; macOS: only reachable when `brew` is already
   detected, same gate as today's inert `brewFormula` field) and a
   consent-friendly elevation story (native polkit dialog / no elevation
   needed at all). Lowest-risk, ships first.
2. **Phase 2 — Windows via `winget`.** Gated on resolving the
   UAC-prompt-origin question in §3.1 first (a short research spike, not
   a large build) — the one platform where "will the OS's own consent
   dialog actually show up correctly" isn't already known from existing
   precedent elsewhere in this codebase.
3. **[DECISION NEEDED] Phase 3 — bootstrapping a MISSING package
   manager itself** (installing Homebrew when a mac has never had it;
   guiding a pre-App-Installer Windows through updating it). This is
   explicitly **not** included in Phases 1-2 above and is flagged, not
   assumed — running Homebrew's official install script is itself a
   "fetch and run a script from the internet" action, a categorically
   bigger trust step than "run a formula install through a package
   manager the user already has and has already used." Recommend
   deciding this separately, after Phase 1-2 ship and the consent-UX
   pattern has real usage to point to, rather than bundling it into v1.
4. **[DECISION NEEDED] Node's Debian/Ubuntu package-name mapping**
   (§3.1's NodeSource-vs-bare-package caveat) — a content/data decision
   for the catalog, flagged so it isn't silently shipped wrong; doesn't
   block the architecture in §3.2-§3.5.

## 7. Touch points (files)

- `agentmux-srv/src/server/install_handlers.rs` (or a new sibling module,
  e.g. `system_install_handlers.rs`) — `SystemInstallCatalog`,
  `toolchain.install_system_tool` / `toolchain.resolve_install_command`
  handlers, reusing `InstallSessionRegistry`'s streaming/cancel plumbing.
- `agentmux-common/src/` — Linux package-manager + distro detection
  (a natural sibling to the existing `toolchain_path.rs`'s "what's on
  this machine" logic).
- `frontend/app/store/rpc-api.ts` (or a dedicated `toolchain-rpc.ts`) —
  `ToolchainInstallSystemToolCommand` / `ToolchainResolveInstallCommandCommand`.
- `frontend/app/view/agent/components/AgentPrereqModal.tsx` — per-row
  Install button + consent panel.
- `frontend/app/view/toolchain/toolchain-view.tsx:286-299` — the existing
  "Install ↗" button gains the conditional real-execution path described
  in §3.5; the copy-command fallback is untouched.
- `frontend/app/view/agent/providers/toolchain-catalog.ts` — add missing
  Windows `installCommand` entries for node/npm/git/docker (currently
  absent, §1); no structural change to `CoreTool` needed beyond what
  `brewFormula` already implies (a parallel `wingetId` field is the
  natural Windows counterpart, mirroring `brewFormula`'s existing shape).
