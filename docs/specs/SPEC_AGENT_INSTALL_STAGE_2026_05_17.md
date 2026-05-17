# SPEC: Agent Install Stage

**Status:** Draft
**Date:** 2026-05-17
**Author:** AgentA
**Related:** [`SPEC_OPENCLAW_AGENT_2026_05_17.md`](./SPEC_OPENCLAW_AGENT_2026_05_17.md) (§6 onboarding, made concrete here for ALL agents), [`SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md`](./SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md) (reuses the same streaming-output primitive)

---

## 0. TL;DR

When the user picks an agent that isn't installed (or whose install is stale / broken), the agent pane's default content becomes a **distinctly-colored Install button** + a **progress feed**. Clicking installs the agent's CLI + runs any post-install steps + verifies. On success the button transitions to **Launch**. Re-clicks during install are idempotent. Cancel rolls back partial state.

This separates "install" from "launch" semantically — today they're conflated in a single click flow, which hides 30–60 second installs behind a silent spinner and (verified today) lets a user queue six parallel `npm install` runs by clicking impatiently.

The install stage is **per-agent declarative** in the provider config, reuses the existing PTY-streaming primitive for visible progress, and supports caching across AgentMux versions to avoid re-installing 373 MB of OpenClaw on every version bump.

---

## 1. Problem

Three concrete failure modes observed today (2026-05-17 session):

### 1.1 Silent first-install

`npm install` runs invisibly under the OpenClaw card's first launch. ~30–60s of nothing in the UI. User clicks again. AgentMux happily queues a second install. By the time it's done, six parallel `npm install` processes are in flight. Same `node_modules` directory. Predictable corruption surface.

### 1.2 Conflated semantics

Today's button: "Connect to OpenClaw". It actually does three things: resolve-or-install CLI, auth-or-not, launch. If install fails the user sees "auth panel didn't appear"; if auth fails they see "Connect button doesn't work"; if launch fails they see "agent doesn't open". The error surface is wrong because the actions are wrong.

### 1.3 Per-version cache duplication

`~/.agentmux/<version>/cli/<agent>/` — fresh dir per AgentMux version bump. Five OpenClaw bumps in a session = 5 × 373 MB = **~1.9 GB** of duplicated `node_modules`. Disk impact is real, and the user pays a full re-install on every minor version label change.

### 1.4 Heterogeneous install recipes hidden in code

| Agent | Install | Post-install needed |
|---|---|---|
| Claude Code | `npm install -g @anthropic-ai/claude-code` | OAuth (interactive) |
| Codex | `npm install -g @openai/codex` | `codex login` (OAuth, interactive) |
| Gemini | `npm install -g @google/genai-cli` (TBD) | OAuth (interactive) |
| OpenClaw | `npm install -g openclaw` | `openclaw config set gateway.mode local` + `openclaw onboard` (interactive) + brain auth |
| Kimi | `pip install kimi-cli` (TBD) | API key |
| Pi | bundled in OpenClaw | — |
| Copilot | `gh extension install …` | GitHub device flow |

Today each agent's install is hard-coded in different code paths (npm in `cli_handlers.rs`, post-install nowhere). The provider config (`providers.rs` + `providers/index.ts`) doesn't describe the post-install graph — so OpenClaw's `gateway.mode = unset` doctor warning has no path to a fix-button.

---

## 2. Goals

- **G1.** Brand-new user → click agent card → Install button (with cost + size + estimated time) → click → visible streamed progress → success → Launch button.
- **G2.** Re-clicking during install is a no-op (or pauses the user-visible bar at most). No queued parallel installs.
- **G3.** Per-agent install recipes are **declarative** in the provider config, not hard-coded in the launch flow.
- **G4.** Post-install steps that need interactive TTY (`openclaw onboard`, `codex login`) reuse the PTY infrastructure shipped with `SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md`.
- **G5.** Install dirs are **shared across AgentMux versions** by default (`~/.agentmux/shared/cli/<agent>/`), with per-version override available for compatibility quirks.
- **G6.** Cancel mid-install reverts to `NOT_INSTALLED` and removes the partial dir.
- **G7.** A first-run successful install ends with the agent able to launch in one more click — no separate "now run doctor" / "now configure" dance.

## 3. Non-goals

- **Not a package manager**. We don't try to update individual deps inside a CLI's `node_modules`. The agent CLI itself is the unit of install/upgrade.
- **Not a global registry replacement**. We use `npm install` for npm-packaged CLIs, `pip install` for pip ones, etc. The recipe just calls the right tool — no AgentMux-internal package server.
- **Not Code-Signing / verification of CLI binaries**. Trust comes from the upstream registry (npmjs.org, PyPI, etc.).
- **Not solving disk-pressure GC**. We add the shared-cache to reduce duplication, but don't add a "GC old installs" pass in v1. Logged as follow-up.
- **Not changing the per-agent OAuth UX** beyond using the install stage as its host. Each provider's OAuth flow remains its own (per the "shared interfaces, distinct flavor" principle).

---

## 4. State machine

Per (agent × identity) pair, the install-and-launch lifecycle is:

```
                                  ┌────────────────────────────────────────┐
                                  │                                        │
                                  ▼                                        │
   user picks agent ─► NOT_INSTALLED ──click Install──► INSTALLING ──fail──┤
                                  │                          │             │
                                  │                          │ ok          │
                                  │                          ▼             │
                                  │                      INSTALLED         │
                                  │                          │             │
                                  │                          ▼             │
                                  │                      AUTH_REQUIRED?  ──┤ no
                                  │                          │             │
                                  │                       yes│             ▼
                                  │                          ▼          READY ─► Launch
                                  │              click "Login with …"     ▲
                                  │                          │             │
                                  │                  AUTHING (PTY login)   │
                                  │                          │             │
                                  │                       ok ▼             │
                                  │                       AUTHED ──────────┘
                                  │
                                  │       ┌───────────── user clicks "Reinstall" ──┐
                                  │       │                                        │
                                  └───────┴────────────────────────────────────────┘
```

State definitions:

| state | description | UI affordance |
|---|---|---|
| `NOT_INSTALLED` | Provider's CLI not present in shared/versioned cache, or version stale | **Install {agent}** button (orange/yellow tint, size estimate, est. time) |
| `INSTALLING` | npm/pip/etc. in progress | Disabled button, streaming progress feed below, **Cancel** button |
| `INSTALL_FAILED` | Install or post-install errored | **Retry** button (same color as Install), error excerpt, full log expandable |
| `INSTALLED` | Binary present; running `<agent> doctor` to determine auth state | Brief "Verifying…" spinner; transitions to AUTH_REQUIRED or AUTHED |
| `AUTH_REQUIRED` | Doctor reports not authed | **Login with {provider}** button (provider-color, per `SPEC_OPENCLAW_AGENT_2026_05_17.md` §3) |
| `AUTHING` | OAuth subprocess running | Streaming feed + URL/copy box (existing PreLaunchAuthPanel WaitingPanel UX) |
| `AUTHED` | Doctor reports authed | Briefly verifies once more, then ready |
| `READY` | Everything passes | **Launch {agent}** button (normal blue, the existing launch UX) |

The state is **persisted in the agent pane's reducer slot** (per [[reference_master_reducer_status]]) so a refresh doesn't drop the user back to NOT_INSTALLED if they already installed.

## 5. Per-agent install recipe (provider config)

Extend `ProviderDefinition` (frontend `providers/index.ts`) and `ProviderConfig` (Rust `providers.rs`) with a declarative install graph:

```ts
interface InstallRecipe {
    /** Heuristic for sizing the install button. Renders as
     *  "Install OpenClaw (~400 MB, ~45 s)". */
    estimatedSizeMb: number;
    estimatedDurationSec: number;

    /** Ordered steps. First failure aborts the recipe and surfaces
     *  the step's error. Each step's stdout/stderr streams into the
     *  install panel via PTY (same pipeline as live-log). */
    steps: InstallStep[];

    /** Once `steps` complete, run this command to verify the install
     *  succeeded end-to-end. Non-zero exit = INSTALL_FAILED. */
    verifyCommand: string[];

    /** Doctor command (parsed for "needs auth" vs "ready" vs
     *  "needs other config"). Run on transition INSTALLED →
     *  AUTH_REQUIRED. */
    doctorCommand: string[];
}

interface InstallStep {
    /** Human label shown in the progress feed: "Installing
     *  npm package openclaw…", "Configuring gateway…", etc. */
    name: string;

    /** Command to run. The first element is the CLI binary path
     *  (resolved by the install runner — `npm`, `pip`, the
     *  agent's own binary post-step-1, …). */
    command: string[];

    /** If true, the step runs under a PTY (for `isatty()`-strict
     *  commands like `openclaw onboard`). Default false (pipes). */
    requiresTty?: boolean;

    /** Env vars to inject. Useful for `OPENCLAW_HOME` etc. */
    env?: Record<string, string>;

    /** If true and the step fails, we stop the recipe. If false,
     *  we log and continue. Default true. */
    required?: boolean;
}
```

### 5.1 OpenClaw recipe (concrete example)

```ts
installRecipe: {
    estimatedSizeMb: 400,
    estimatedDurationSec: 45,
    steps: [
        {
            name: "Installing OpenClaw npm package…",
            command: ["npm", "install", "openclaw@latest", "--prefix", "{installDir}"],
        },
        {
            name: "Configuring local gateway mode…",
            command: ["{cli}", "config", "set", "gateway.mode", "local"],
        },
        // `openclaw onboard` is interactive; treat as a separate
        // user-driven step (don't auto-run). The user clicks
        // "Run onboarding" once the install completes and `doctor`
        // surfaces the need. Phase β.
    ],
    verifyCommand: ["{cli}", "--version"],
    doctorCommand: ["{cli}", "doctor", "--json"],
}
```

`{installDir}` is `~/.agentmux/shared/cli/openclaw/` (per §7) and `{cli}` is `{installDir}/node_modules/.bin/openclaw{.cmd}`.

### 5.2 Claude Code recipe

```ts
installRecipe: {
    estimatedSizeMb: 250,
    estimatedDurationSec: 20,
    steps: [
        {
            name: "Installing Claude Code…",
            command: ["npm", "install", "@anthropic-ai/claude-code@latest", "--prefix", "{installDir}"],
        },
    ],
    verifyCommand: ["{cli}", "--version"],
    doctorCommand: ["{cli}", "auth", "status", "--json"],
}
```

(Today's `authCheckCommand` becomes `doctorCommand`. Authoring is just renaming + adding the `installRecipe` wrapper.)

### 5.3 Auth recipe lives separately

`authLoginCommand` (already exists) stays as-is — it's the OAuth/interactive login the user triggers after INSTALLED. Install recipe ends at INSTALLED; auth is a separate phase.

---

## 6. UI flow

### 6.1 Two surfaces: a card in the pane + a modal for the work

The default agent-pane view when the picked agent isn't installed renders a **single tinted Install button** + a one-line description. Clicking it opens an **Install Modal** (sibling to today's `AgentLaunchModal.tsx`) that hosts the live xterm-rendered install feed. The pane behind stays where it is.

This pattern matches the launch UX: a small clickable affordance in the pane, the actual work happens in a modal that can be sized, dragged, kept open while the user does something else, or dismissed without canceling the underlying process.

#### Default pane view — `NOT_INSTALLED`

```
┌── agent pane ─────────────────────────────────────────────────┐
│                                                                │
│   🦞 OpenClaw                                                  │
│   Open-source personal AI assistant — model-agnostic           │
│                                                                │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │  ⚡ Install OpenClaw   (~400 MB, ~45 s)                 │  │
│   └─────────────────────────────────────────────────────────┘  │
│                                                                │
│   Bundled: openclaw CLI, OpenAI + Anthropic + Google +         │
│   Mistral provider SDKs, playwright (browser control).         │
│   Stored at ~/.agentmux/shared/cli/openclaw/                   │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

The Install button is the **only** interactive affordance in this state — distinct color (suggest amber `#d9923a` to read as "do this thing first") so users don't confuse it with the regular Launch button (blue). No identity dropdown, no message composer.

#### Install Modal — opens on click, persists across pane navigation

The modal's layout mirrors `AgentLaunchModal.tsx`:

```
┌── Install OpenClaw ──────────────────────────────────────────┐ ✕
│                                                               │
│  Step 1 of 2: Installing npm package openclaw                 │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ ▌                                                      │   │
│  │ openclaw@2026.5.12                                     │   │
│  │ added 487 packages in 12s                              │   │
│  │ npm warn deprecated readable-stream@…                  │   │
│  │ ▌                                                      │   │
│  └────────────────────────────────────────────────────────┘   │
│  (xterm.js live view — handles ANSI, progress bars, color)    │
│                                                               │
│  Disk: 287 MB / ~400 MB estimated                             │
│  Elapsed: 0:23                                                │
│                                                               │
│              [ Cancel ]                  [ Background ]       │
└───────────────────────────────────────────────────────────────┘
```

Key affordances:
- **xterm.js inside the modal** — full terminal rendering so npm's `[==>] 50%` progress lines, ANSI colors, and `\r`-overwriting redraws look right. Same xterm we use for terminal panes (`agentmux-srv/src/backend/blockcontroller/shell.rs` plumbing).
- **Step indicator** at top tells the user which step of the recipe is running.
- **Disk + elapsed**, surfaced so the user understands the bar isn't stuck.
- **Cancel** kills the install + rolls back partial state (§9.3).
- **Background** closes the modal but the install keeps running. The pane's Install button updates to "Installing… (12 MB / 400 MB)" with a progress hint so the user can re-open the modal anytime.
- **Modal close (✕)** is equivalent to Background — install continues, doesn't cancel.

#### `INSTALL_FAILED` modal variant

```
┌── Install OpenClaw — failed ─────────────────────────────────┐ ✕
│                                                               │
│  ⚠  Step 1 of 2 failed: npm install                           │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ npm ERR! code ETIMEDOUT                                │   │
│  │ npm ERR! errno -4039                                   │   │
│  │ npm ERR! syscall connect                               │   │
│  │ ▌                                                      │   │
│  └────────────────────────────────────────────────────────┘   │
│                                                               │
│  Possible cause: network timeout. Retry once you're online.   │
│                                                               │
│              [ View full log ]    [ Retry ]                   │
└───────────────────────────────────────────────────────────────┘
```

#### After success — INSTALLED → AUTH_REQUIRED

Modal **auto-closes** on a successful end-of-recipe + `doctor` parse. Pane transitions to:

```
┌── agent pane ─────────────────────────────────────────────────┐
│                                                                │
│   ✓ OpenClaw installed (391 MB)                                │
│                                                                │
│   Sign in to enable a backing model:                           │
│                                                                │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │  Login with OpenAI (ChatGPT)                            │  │
│   └─────────────────────────────────────────────────────────┘  │
│                                                                │
│   More providers (Anthropic, Google, Mistral) — later.         │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

After AUTHED → READY: the existing AgentLaunchModal opens with "Launch OpenClaw" (no change to existing UX past this point).

### 6.2 xterm.js inside the modal

The install modal renders the install subprocess through an xterm.js instance, NOT the plain `ChunkList` we use for the bash tool overlay. Reasoning:

- `npm install` output is heavily ANSI-encoded (progress meters, color-coded warnings, redraws). Plain text rendering looks worse than the user's expectation for an install screen.
- xterm.js is already in our frontend deps and powers the terminal pane.
- The bash tool overlay's `ChunkList` is the right call for "agent did `ls`, show me the output as a chunk list." Install is "this is a terminal — let it draw itself."
- ANSI strip in the bashwrap path is conscious lossy normalisation for the chat conversation. For an install screen we want the opposite — keep everything, render it.

Mechanically:
- Backend streams raw PTY bytes (no strip) on `install_chunk` WPS events scoped to `agent:<provider>` (no per-block scope since install is agent-level, not block-level).
- Frontend feeds bytes into the modal's xterm.js instance via `term.write(bytes)`. Identical pattern to terminal pane.
- Cap scrollback at 8 000 lines (terminal-default) so a chatty install doesn't OOM the renderer.

### 6.3 Re-click idempotency

Re-clicks on the Install button when the install is already in flight (modal open OR backgrounded) re-open the modal instead of starting a second install. The reducer (per §8) holds a single in-flight `installToken` per agent; subsequent `StartInstall` dispatches with that token already present are a visual no-op (modal pops to front, that's all).

---

## 7. Shared cache across AgentMux versions

### 7.1 The 1.9 GB problem

Today: `~/.agentmux/<version>/cli/openclaw/` — 373 MB per version. After five `bump patch` builds today, the user has 1.9 GB of duplicated `node_modules` on disk.

### 7.2 Proposed layout

```
~/.agentmux/
├── shared/
│   └── cli/
│       ├── openclaw/      ← single source of truth, ~373 MB
│       ├── claude/        ← ~219 MB
│       └── …
└── versions/
    └── 0.33.903/
        └── cli/
            └── openclaw/  → SYMLINK to ../../../shared/cli/openclaw
```

Each `~/.agentmux/<version>/cli/<agent>/` becomes a symlink (or NTFS junction on Windows) into `~/.agentmux/shared/cli/<agent>/`. Install runs in `shared/`. Subsequent version bumps reuse it.

### 7.3 Version compatibility

What if openclaw v2 introduces a breaking change and AgentMux 0.33.910 needs v2 while 0.33.909 needs v1? Provider config can opt in to per-version isolation:

```ts
installRecipe: {
    cacheScope: "shared" | "per-version";   // default: "shared"
    // …
}
```

For v1 of this spec, default `shared` for every provider. Per-version isolation as an escape hatch.

### 7.4 Migration

On first launch with the new layout: if `~/.agentmux/<version>/cli/<agent>/` exists as a real directory (not a symlink), move it to `~/.agentmux/shared/cli/<agent>/` and replace with a symlink. This is a one-shot migration; subsequent versions just create the symlink at version-creation time.

---

## 8. Reducer + state

Per [[reference_master_reducer_status]], all per-pane lifecycle state lives in a slot store. Add a new slice `agent-install-store`:

```ts
interface AgentInstallState {
    perAgent: Map<string, AgentInstallEntry>;   // keyed by agentId
}

interface AgentInstallEntry {
    state:
        | { kind: "not_installed" }
        | { kind: "installing"; token: string; steps: InstallStepProgress[]; log: LogChunks }
        | { kind: "install_failed"; error: string; logExcerpt: string; log: LogChunks }
        | { kind: "installed_verifying" }
        | { kind: "auth_required" }
        | { kind: "authing"; sessionId: string; authUrl?: string }
        | { kind: "authed" }
        | { kind: "ready" };
}

interface InstallStepProgress {
    name: string;
    status: "pending" | "running" | "ok" | "failed";
}
```

Reducer commands:

- `CheckAgentInstalled { agentId }` — opens the pane, runs the recipe's verify command, sets initial state.
- `StartInstall { agentId, token }` — transitions to INSTALLING. Idempotent on same token.
- `InstallChunk { agentId, chunk }` — append to log.
- `InstallStepDone { agentId, stepIndex, ok }` — advance UI marker.
- `InstallComplete { agentId }` — transitions to INSTALLED_VERIFYING.
- `InstallFailed { agentId, error }` — transitions to INSTALL_FAILED.
- `CancelInstall { agentId }` — transitions to NOT_INSTALLED, fires backend cancel.
- `DoctorResult { agentId, kind: "auth_required" | "authed" | "needs_config", details? }` — branches state.

Auth-state transitions integrate with existing `auth-flow-controller.ts` — AUTHING is a thin wrapper that delegates.

---

## 9. Backend

### 9.1 New RPC commands

| Command | Purpose |
|---|---|
| `install.start` | Begin an install recipe. Returns `installToken`. Idempotent on token. |
| `install.cancel` | Abort an in-flight install + roll back partial state. |
| `install.poll` | Optional — used if WS isn't reliable; otherwise events stream over existing chunk pipeline. |
| `install.doctor` | Run the recipe's doctor command; parse output to one of `auth_required` / `authed` / `needs_config` / `unknown`. |

### 9.2 Streaming

PTY-spawn each install step (via `portable_pty`, same as `spawn_auth_cli_pty`). Lines fed through `record_install_line` (analogous to `record_line` in auth_session) → events on `install_chunk` WPS event scoped to the agent block.

### 9.3 Atomic install dir

Install to `{cacheDir}/{agent}.tmp/`. On verify success, atomic rename to `{cacheDir}/{agent}/`. On failure or cancel, rm-rf the tmp dir.

### 9.4 Cancel signal

Same cancel-handle pattern as `auth_session_manager`: `install.cancel` sends a oneshot the install-task watches. On signal, child is killed via PID + the tmp dir is removed.

---

## 10. Risks + mitigations

### 10.1 npm install hung / network slow

Recipe step has an implicit deadline (no timer in our code — `npm install` itself fails on its own connect timeout, ~30s default). If the user cancels, we kill via PID and rm-rf. **No timer added in our code** per `feedback_no_timers_or_delays.md` — the install network primitives have their own deadlines, we just propagate cancel signals.

### 10.2 Symlink permissions on Windows

NTFS symlinks require admin or "Developer Mode" enabled. Mitigation: use NTFS junctions (don't require elevation) via `cmd /c mklink /J`. Junctions are functionally equivalent for dir-only links.

### 10.3 Shared cache + version drift

Two AgentMux versions on the same machine pointing at `~/.agentmux/shared/cli/openclaw/`: one upgrades the cache, the other might break. Mitigation: see §7.3 `cacheScope: "per-version"` escape hatch. Default `shared` for v1 — most CLIs are forward-compatible.

### 10.4 Re-install during running session

User has an OpenClaw agent open, then clicks "Reinstall" elsewhere. Currently-running agent now uses a possibly-deleted binary. Mitigation: detect any open panes using this CLI before allowing reinstall; surface a "Close all OpenClaw panes first?" confirmation.

### 10.5 First-install offline

User clicks Install with no network → `npm install` fails. We surface the error in the panel + retry button. No silent loop, no timer — the user reads the error and acts.

---

## 11. Phased rollout

### Phase α — scaffold + Install button + xterm modal (1 PR)
- New `AgentInstallCard.tsx` rendered when the agent isn't installed (the tinted button + description, in the agent pane's default view).
- New `AgentInstallModal.tsx` (sibling to `AgentLaunchModal.tsx`) hosting the xterm.js live install feed.
- New `agent-install-store` reducer slice with the state machine (`NOT_INSTALLED → INSTALLING → INSTALL_FAILED|INSTALLED`), most transitions stubbed past INSTALLED.
- Single-step recipe (just `npm install`, no post-install steps yet) reusing existing `ResolveCli` plumbing.
- Backend streams raw PTY bytes on `install_chunk` WPS events; modal writes them into xterm via `term.write`.
- Re-click idempotency: clicking the Install button while modal is open re-focuses it, doesn't start a second install.
- No shared cache yet — still per-version.

**Exit criteria:** user clicks the OpenClaw card → sees the tinted Install button → click → modal opens with xterm-rendered live npm install → reaches INSTALLED → modal auto-closes → pane shows "Login with OpenAI" follow-up. No more "click 6 times, queue 6 npm installs".

### Phase β — declarative install recipes
- `installRecipe` added to provider config (TS + Rust).
- Multi-step support, with verify + doctor.
- OpenClaw recipe gets `config set gateway.mode local` as a second step.

### Phase γ — shared cache
- `~/.agentmux/shared/cli/` becomes the default. Per-version dirs become symlinks/junctions.
- One-shot migration on first launch (move existing dirs into shared, replace with junctions).

### Phase δ — interactive post-install (TTY-required steps)
- Steps with `requiresTty: true` spawn under PTY (reuse `spawn_auth_cli_pty` pattern).
- OpenClaw's `onboard` flow becomes a button: "Run onboarding (~30 s)".

### Phase ε — reinstall + update
- "Reinstall" button on a ready/authed agent.
- "Update available" badge if upstream version differs from installed.

---

## 12. Open questions

1. **Where does the Install button live spatially** when the agent picker (forge) is browseable? On the pane after pick, vs inline in the picker card? Recommend pane-after-pick — picker stays a chooser, install is a commitment.

2. **Should the Install button surface required environment** (Node.js version, Python version)? OpenClaw needs Node ≥ 20 per its package.json `engines`. AgentMux doesn't currently check. Lean: add a "Requirements check" step ahead of install steps; fail-loud with a "Node ≥ 20 required, you have X" if missing.

3. **Per-identity install isolation?** Today every identity shares one `~/.agentmux/<version>/cli/openclaw/`. If two identities want different OpenClaw versions, they can't. Likely fine; flag as known limitation.

4. **Streaming-output limit during install** — `npm install` can output thousands of lines (deprecated warnings, etc.). Cap at N lines or last-N-lines-on-failure? Reuse the same scrollback cap as the bash overlay (sensible 1024-line default).

5. **Estimate accuracy** — `estimatedSizeMb` and `estimatedDurationSec` are hand-coded. Should they get refreshed from telemetry (median of last N installs on this OS)? Out of scope for v1.

6. **Failure detail** — `INSTALL_FAILED` shows an error excerpt; the full log stays expandable. Where? Inline below the Retry button? Modal? Lean: inline, collapsed by default, expand-on-click — matches the live-log tool overlay UX.

---

## 13. Test plan

### 13.1 Happy path (first install)
1. Fresh AgentMux (no `~/.agentmux/shared/cli/` yet).
2. Open agent picker, pick OpenClaw.
3. See Install button with "~400 MB, ~45 s".
4. Click Install → progress feed streams `npm install` output.
5. Within ~60 s, transitions to AUTH_REQUIRED.
6. Click "Login with OpenAI" → existing PreLaunchAuthPanel.WaitingPanel UX.
7. Complete OAuth → AUTHED → READY → Launch button.

### 13.2 Re-click idempotency
1. Click Install. Immediately click again (and again, and again). Console + reducer log: at most one install command dispatched.

### 13.3 Cancel mid-install
1. Click Install. After ~5s, click Cancel.
2. State returns to NOT_INSTALLED.
3. `~/.agentmux/<version>/cli/openclaw.tmp/` removed; no `node_modules` left behind.

### 13.4 Install failure
1. Disconnect network. Click Install.
2. npm install fails (network error in its stderr).
3. State → INSTALL_FAILED. Error excerpt visible. Retry button works after network restored.

### 13.5 Shared cache (Phase γ)
1. Install OpenClaw on AgentMux 0.33.910.
2. Bump to 0.33.911, launch.
3. Pick OpenClaw — should be already INSTALLED (verify command runs, doctor runs, jumps to AUTH_REQUIRED or AUTHED). No re-install.

### 13.6 Re-install during open pane
1. OpenClaw pane open. Open a second pane, hit Reinstall (Phase ε).
2. Confirmation modal: "Close all OpenClaw panes first?" — Cancel returns to normal; Confirm closes panes then reinstalls.

---

## 14. References

### Code (current state, pre-spec)
- `frontend/app/view/agent/providers/index.ts` — `ProviderDefinition` (grows `installRecipe` field)
- `agentmux-srv/src/backend/providers.rs` — `ProviderConfig` (Rust mirror grows the same field)
- `agentmux-srv/src/server/cli_handlers.rs` — existing `npm install` + `ResolveCli` (becomes one step of the recipe)
- `agentmux-srv/src/server/identity_handlers.rs` — `spawn_auth_cli_pty` (reused for `requiresTty: true` install steps)
- `agentmux-srv/src/identity/auth_session.rs` — pattern for `install_session_manager` (mirrors auth-session machinery)
- `frontend/app/view/agent/components/PreLaunchAuthPanel.tsx` — AUTH_REQUIRED → AUTHING transitions already wired here; keep them, just relocate the entry point into the new install panel
- `frontend/app/view/agent/components/AgentInstallCard.tsx` — NEW (the tinted Install button + description in the pane's default view)
- `frontend/app/view/agent/components/AgentInstallModal.tsx` — NEW (modal with xterm.js live feed; sibling to `AgentLaunchModal.tsx`)
- `agentmux-srv/src/identity/install_session.rs` — NEW (analogous to `auth_session.rs`; tracks in-flight installs, holds cancel tokens, emits `install_chunk` events)
- xterm.js terminal-pane shell (already in deps) — reused for the modal feed

### Prior specs
- [`SPEC_OPENCLAW_AGENT_2026_05_17.md`](./SPEC_OPENCLAW_AGENT_2026_05_17.md) — §6β "onboarding UX" generalized here
- [`SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md`](./SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md) — PTY-streaming primitive reused for install progress
- [`SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md`](./SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md) — auto-expand-while-running pattern; the same idea applies to install panel during INSTALLING
- [`SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md`](./SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md) — install state persists across pane reopen via the same snapshot mechanism

### Memory
- [[feedback_no_timers_or_delays]] — no fixed-duration grace windows for npm install (let npm's own timeouts fire)
- [[feedback_dont_measure_the_meter]] — install progress streaming must not loop-back into the auth or launch streaming paths
- [[reference_master_reducer_status]] — install state lives in a per-pane slot store, audit-friendly
