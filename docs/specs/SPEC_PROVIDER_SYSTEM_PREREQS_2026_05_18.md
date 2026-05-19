# SPEC: Provider System-Tool Prerequisites

**Status:** Draft
**Date:** 2026-05-18
**Author:** AgentA
**Related:**
- `frontend/app/view/agent/providers/index.ts` (provider definitions)
- `agentmux-cef/src/commands/providers.rs::check_nodejs_available` (existing single-tool check)
- Anthropic Claude Code issue [#29898](https://github.com/anthropics/claude-code/issues/29898) (the runtime error this spec eliminates)

---

## 0. TL;DR

Some CLI providers AgentMux supports require system tools beyond Node.js to actually run. Claude Code, for instance, errors at session start with:

> `Error: Git is required but was not found. Install git and try again.`

We don't check for git today, so the user sees this stderr from Claude Code itself rather than an AgentMux notice. Same gap exists in principle for any future provider that depends on `gh`, `python`, etc.

Fix: declare each provider's `systemPrereqs` as part of `ProviderDefinition`. Probe the system at install / launch time via a new `resolve_prereqs` RPC. Show a friendly pre-launch banner listing any missing tools with their official download URLs as clickable links.

---

## 1. Problem

Today's flow with Claude Code on a machine without git:

1. User clicks the Claude Code card in the agent picker.
2. Install modal runs `npm install @anthropic-ai/claude-code` — succeeds (no git dep at install time).
3. User clicks Continue to Launch.
4. Agent pane spawns `claude` CLI.
5. Claude Code starts a local session, calls git, and prints to stderr:
   > `Error: Git is required but was not found. Install git and try again.`
6. User sees that error in the agent terminal — no AgentMux UI explaining what to do.

Same shape for any provider that internally relies on a non-bundled system tool.

---

## 2. Best-practices comparison

- **VS Code extensions** with system-tool deps surface a notification on activation: "This extension requires `<tool>`. Install from `<url>`." Click → opens the URL in browser. Most check on activation rather than at every command.
- **Tauri's auto-updater** does platform-specific tool probes during init.
- **Homebrew** declares formula `depends_on` and refuses to install if missing — too rigid for our case (we want to inform, not block install).

The pattern that fits AgentMux: check at **install / launch time** and surface a banner with install URLs. Don't hard-block the launch — the user may have the tool under a non-standard path that `where`/`which` can't find. Allow override with "Launch anyway."

---

## 3. Proposed architecture

### 3.1 Declare prereqs on the provider

Add to `ProviderDefinition` (`frontend/app/view/agent/providers/index.ts`):

```ts
export interface SystemPrereq {
    /** Binary name to look up via `where` (Windows) / `which` (Unix). */
    tool: string;
    /** Human-readable display name shown in the banner.
     *  Defaults to `tool` if omitted. */
    label?: string;
    /** Per-platform install URLs. Each one is a curated landing page,
     *  not the raw downloads index — so the link feels intentional. */
    installUrls: {
        windows: string;
        macos: string;
        linux: string;
    };
    /** When true, mark the prereq as launch-blocking in the UI.
     *  When false, show as a warning (user can Launch anyway).
     *  Defaults to true — most prereqs are hard reqs at runtime. */
    blocking?: boolean;
}

export interface ProviderDefinition {
    // ...existing fields...
    /** System tools the provider needs at runtime (beyond Node, which
     *  is checked separately by `check_nodejs_available`). */
    systemPrereqs?: SystemPrereq[];
}
```

Initial population:

| Provider | Prereqs |
|---|---|
| `claude` | `git` (issue #29898) |
| `codex` | none (OpenAI Codex CLI doesn't shell out to system tools) |
| `gemini` | none |
| `openclaw` | `git` (built on the Codex harness which doesn't need it, but openclaw model auth + project context use git) |
| `kimi` | none |
| `copilot` | `gh` (GitHub Copilot CLI wraps `gh`) |
| `pi` | none |

This list is conservative — if a provider doesn't *consistently* fail without a tool, leave it off. False-positive banners are worse than no banner.

### 3.2 Probe RPC

New `RpcApi.ResolvePrereqsCommand` backed by `agentmux-srv/src/server/cli_handlers.rs::resolve_prereqs`:

```rust
#[derive(Deserialize)]
struct ResolvePrereqsReq {
    tools: Vec<String>,
}

#[derive(Serialize)]
struct ResolvePrereqsRsp {
    /// One entry per requested tool, preserving order. `path` is the
    /// resolved absolute path (Some) or None if not found.
    results: Vec<PrereqResult>,
}

#[derive(Serialize)]
struct PrereqResult {
    tool: String,
    found: bool,
    path: Option<String>,
}
```

Probe via `where {tool}` (Windows) / `which {tool}` (Unix). Use the **path-only** form so we don't accidentally execute the tool. Cache per-renderer-process for the session (PATH doesn't change while AgentMux is running).

### 3.3 UI: pre-launch prereq banner

When the user clicks an agent card whose provider has `systemPrereqs`:

1. AgentPicker calls `resolve_prereqs(provider.systemPrereqs.map(p => p.tool))` BEFORE opening the install or launch modal.
2. If any required tools are missing → open a new "Prerequisites" modal kind in the existing `TabModalLayer` showing a list:

   ```
   Claude Code needs some tools that aren't installed.

   ⚠ Git — not found
        Install Git for Windows ↗  (https://git-scm.com/download/win)

   [ Launch anyway ]   [ Cancel ]
   ```

3. "Cancel" → close.
4. "Launch anyway" → proceed to the install modal (or directly to launch if already installed).

If all prereqs present → fall through to the existing install/launch flow with no extra step.

The banner links use `getApi().openExternal(url)` (existing CEF IPC for opening URLs in the system browser) so the user lands on the official downloads page in their default browser.

### 3.4 Platform detection for the URL

The `useSystemPlatform()` hook already exists in `frontend/app/store/global.ts` (returns `"win32" | "darwin" | "linux"`). The banner picks the matching URL from `installUrls.{windows,macos,linux}`.

### 3.5 Banner copy

```
Title:    "Install required tools to use {displayName}"
Subtitle: "{displayName} needs the following tools. Install them and restart AgentMux."

Per-tool row:
   ⚠ <label> — not found
   ↗ <Install link> (anchor text is platform-specific, e.g. "Install Git for Windows")

Footer:
   [ Cancel ]   [ Launch anyway ]   [ Refresh ] (re-probe after user installs)
```

A "Refresh" button re-runs the RPC so a user who installs git in another terminal can re-check without re-opening AgentMux.

---

## 4. Implementation plan

### Phase α — claude + openclaw only

1. `frontend/app/view/agent/providers/index.ts` — add `SystemPrereq` interface + `systemPrereqs` field; populate for `claude` and `openclaw`.
2. `agentmux-srv/src/server/cli_handlers.rs` — new `resolve_prereqs` RPC handler.
3. Auto-generated RPC bindings — re-run codegen (`task generate-ts`?).
4. `frontend/app/view/agent/components/AgentPrereqModal.tsx` — new component using modal-v2 chrome. Reuses the same `useTabModal` pattern as install/launch.
5. `frontend/app/tab/tab-modal.ts` — new `kind: "agent-prereqs"` request shape.
6. `frontend/app/view/agent/components/AgentPicker.tsx` — between card click and install-modal open, call `resolve_prereqs`; if anything missing + blocking, route to the prereq modal first.

### Phase β — extend

7. Refresh-button wiring.
8. Add `copilot` → `gh` once we ship that provider.

---

## 5. Curated install URLs

Per the user's "good looking links to proper locations":

| Tool | Windows | macOS | Linux |
|---|---|---|---|
| `git` | https://git-scm.com/download/win | https://git-scm.com/download/mac | https://git-scm.com/download/linux |
| `gh` | https://cli.github.com/ | https://cli.github.com/ | https://cli.github.com/ |

These are the official landing pages each project maintains. The Windows git page in particular leads with the prominent download button — best signal for our users.

---

## 6. Test plan

- [ ] On a machine without git installed: click Claude Code → prereq modal opens listing Git → click "Install Git for Windows" → opens https://git-scm.com/download/win in default browser → user installs → clicks Refresh → modal updates to "all prereqs satisfied" → can launch.
- [ ] On a machine WITH git: click Claude Code → prereq modal does NOT open; flow proceeds straight to install/launch.
- [ ] Click "Launch anyway" with git missing → install/launch proceeds → user sees Claude Code's `Git is required` stderr in the agent pane (best we can do — the agent itself owns that flow now).
- [ ] Provider with no `systemPrereqs` → no extra modal, no extra RPC call (skip the probe).
- [ ] Cancel from prereq modal → returns to picker, no install attempted.

---

## 7. Acceptance criteria

1. New `SystemPrereq` field on `ProviderDefinition`, populated for `claude` and `openclaw` with `git`.
2. `resolve_prereqs` RPC returns accurate found/path for each tool on Windows, macOS, Linux.
3. Pre-launch prereq modal opens only when prereqs are missing AND blocking.
4. Install links open the platform-appropriate URL in the user's default browser.
5. "Launch anyway" overrides the modal and proceeds.
6. "Refresh" re-probes without closing.
7. No extra latency on launch for providers without prereqs (RPC skipped).

---

## 8. Out of scope

- **Bundled tools** in the portable (e.g. shipping our own `git.exe`). Adds ~50MB and creates an upgrade-skew problem (users may run a different system git in terminals). Separate spec if requested.
- **Auto-install** of missing tools via package managers (winget, brew, apt). Cross-platform package-manager invocation is its own can of worms.
- **Per-provider prereqs configurable by the user** (e.g. "I know my git is at `D:\portable\git\bin\git.exe`"). Could add `prereqOverrides: { tool: path }` to settings later.
- **Probing during AgentMux startup** to surface a global "missing tools" notice. Not a launcher concern; per-provider lazy probe is enough.
