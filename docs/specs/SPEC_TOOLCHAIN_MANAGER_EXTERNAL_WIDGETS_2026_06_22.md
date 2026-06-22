# Toolchain Manager — External Widgets Extension

**Date:** 2026-06-22
**Status:** Spec / Ready to implement
**Extends:** `docs/specs/SPEC_TOOLCHAIN_MANAGER_2026-06-15.md` (P2/P3 phases)
**Scope:**
- `frontend/app/view/agent/providers/toolchain-catalog.ts` — add Python + uv
- `frontend/app/view/agent/providers/widget-catalog.ts` — new file
- `frontend/app/modals/toolchain-modal.tsx` — new "External Widgets" section
- `agentmux-srv/src/server/install_handlers.rs` — pip install handler
- `agentmux-srv/src/server/widget_health_handlers.rs` — new file, HTTP health check RPC

---

## Goal

Extend the Toolchain Manager (hamburger → Toolchain Manager) so that users can
detect, install, launch, and open any supported external widget — all from a
single panel, without leaving AgentMux.

**End state:**

```
┌─ Toolchain Manager ─────────────────────────────────────┐
│  Environment        PATH: login-shell ✓                 │
│  Core Tools         Node ✓  npm ✓  Git ✓  Python ✓     │
│  Agent CLIs         Claude ✓  Gemini ✓  ...             │
│  External Widgets                                        │
│    ComfyUI    [Installed ✓] [Running ✓]  [Open Pane]   │
│    JupyterLab [Installed ✓] [Running ✗]  [Launch] [Open]│
│    Open WebUI [Not installed]            [Install]      │
│    LangFlow   [Not installed]            [Install]      │
└─────────────────────────────────────────────────────────┘
```

---

## Phase A — Python in Core Tools

### A1. Add Python and uv to `toolchain-catalog.ts`

Extend `CORE_TOOLS` with two new entries after Docker:

```typescript
{
    id: "python",
    cliCommand: "python3",        // "python" fallback handled by backend
    label: "Python",
    icon: "snake",                // fa-solid fa-snake
    minVersion: "3.10",
    description: "Required runtime for ComfyUI, JupyterLab, MLflow, and other AI tools.",
    docsUrl: "https://www.python.org/downloads/",
    installUrls: {
        windows: "https://www.python.org/downloads/windows/",
        macos:   "https://www.python.org/downloads/macos/",
        linux:   "https://www.python.org/downloads/source/",
    },
    installCommand: {
        macos:  "brew install python@3.12",
        linux:  "sudo apt install -y python3 python3-pip python3-venv",
    },
    brewFormula: "python@3.12",
},
{
    id: "uv",
    cliCommand: "uv",
    label: "uv",
    icon: "bolt",
    optional: true,
    description: "Fast Python package/project manager. Replaces pip+venv — 10–100× faster installs.",
    docsUrl: "https://docs.astral.sh/uv/",
    installUrls: {
        windows: "https://docs.astral.sh/uv/getting-started/installation/",
        macos:   "https://docs.astral.sh/uv/getting-started/installation/",
        linux:   "https://docs.astral.sh/uv/getting-started/installation/",
    },
    installCommand: {
        windows: "powershell -c \"irm https://astral.sh/uv/install.ps1 | iex\"",
        macos:   "brew install uv",
        linux:   "curl -LsSf https://astral.sh/uv/install.sh | sh",
    },
    brewFormula: "uv",
},
```

**Backend note:** `resolvecli` RPC already handles `which python3` / `which python`
fallback. No backend change needed for detection. For version parsing, Python
outputs `Python 3.12.3` — the existing version-strip regex covers this.

---

## Phase B — Widget Catalog

### B1. New file: `frontend/app/view/agent/providers/widget-catalog.ts`

Defines the `ExternalWidget` interface and the initial catalog of 10 widgets.

```typescript
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export type Platform = "windows" | "macos" | "linux";

/** How the widget is installed onto the user's machine. */
export type InstallMethod =
    | { kind: "pip";  package: string; extraArgs?: string[] }
    | { kind: "npm";  package: string; version?: string }
    | { kind: "brew"; formula: string }
    | { kind: "manual" };  // user installs themselves; we only detect

/** How the widget's local server is launched. */
export type LaunchMethod =
    | { kind: "python-module"; module: string; args?: string[] }
    | { kind: "cli"; command: string; args?: string[] }
    | { kind: "none" };  // can't launch — must be started by user

export interface ExternalWidget {
    /** Stable ID — becomes the view type string for the pane. */
    id: string;
    label: string;
    icon: string;                    // fa-solid fa-<icon>
    description: string;
    /** Runtime prerequisite IDs from CORE_TOOLS. */
    requires: string[];              // e.g. ["python"] or ["node", "npm"]
    /** CLI binary to probe on PATH (or in managed install dir). */
    cliCommand?: string;
    /** How to install. */
    install: InstallMethod;
    /** How to launch the local server. */
    launch: LaunchMethod;
    /** Default port for health check. */
    defaultPort: number;
    /** Path to poll for server liveness. */
    healthCheckPath: string;         // e.g. "/system_stats"
    /** URL path to embed (relative to http://127.0.0.1:<port>). */
    embedPath: string;               // usually "/"
    license: string;
    docsUrl: string;
    /** Whether AgentMux manages the process lifecycle (spawn on Launch click,
     *  kill on AgentMux exit). False = user manages externally. MVP: false. */
    managedProcess: boolean;
}

export const EXTERNAL_WIDGETS: ExternalWidget[] = [
    {
        id: "comfyui",
        label: "ComfyUI",
        icon: "diagram-project",
        description: "Node-based generative AI workflow editor. Stable Diffusion, Flux, video, audio.",
        requires: ["python"],
        cliCommand: "comfyui",
        install: { kind: "pip", package: "comfyui" },
        launch: { kind: "python-module", module: "comfyui", args: ["--listen", "127.0.0.1"] },
        defaultPort: 8188,
        healthCheckPath: "/system_stats",
        embedPath: "/",
        license: "GPL-3.0",
        docsUrl: "https://docs.comfy.org/",
        managedProcess: false,
    },
    {
        id: "jupyterlab",
        label: "JupyterLab",
        icon: "book-open",
        description: "Interactive Python notebook environment.",
        requires: ["python"],
        cliCommand: "jupyter",
        install: { kind: "pip", package: "jupyterlab" },
        launch: { kind: "cli", command: "jupyter", args: ["lab", "--no-browser", "--port=8888"] },
        defaultPort: 8888,
        healthCheckPath: "/api",
        embedPath: "/",
        license: "BSD-3-Clause",
        docsUrl: "https://jupyterlab.readthedocs.io/",
        managedProcess: false,
    },
    {
        id: "open-webui",
        label: "Open WebUI",
        icon: "comments",
        description: "Local chat UI for Ollama and OpenAI-compatible models.",
        requires: ["python"],
        cliCommand: "open-webui",
        install: { kind: "pip", package: "open-webui" },
        launch: { kind: "cli", command: "open-webui", args: ["serve"] },
        defaultPort: 8080,
        healthCheckPath: "/health",
        embedPath: "/",
        license: "MIT",
        docsUrl: "https://docs.openwebui.com/",
        managedProcess: false,
    },
    {
        id: "langflow",
        label: "LangFlow",
        icon: "code-branch",
        description: "Visual drag-and-drop LLM pipeline builder.",
        requires: ["python"],
        cliCommand: "langflow",
        install: { kind: "pip", package: "langflow" },
        launch: { kind: "cli", command: "langflow", args: ["run", "--host", "127.0.0.1"] },
        defaultPort: 7860,
        healthCheckPath: "/health",
        embedPath: "/",
        license: "Apache-2.0",
        docsUrl: "https://docs.langflow.org/",
        managedProcess: false,
    },
    {
        id: "flowise",
        label: "Flowise",
        icon: "sitemap",
        description: "Visual LangChain/LlamaIndex pipeline builder.",
        requires: ["node", "npm"],
        cliCommand: "flowise",
        install: { kind: "npm", package: "flowise" },
        launch: { kind: "cli", command: "flowise", args: ["start"] },
        defaultPort: 3000,
        healthCheckPath: "/api/v1/ping",
        embedPath: "/",
        license: "Apache-2.0",
        docsUrl: "https://docs.flowiseai.com/",
        managedProcess: false,
    },
    {
        id: "mlflow",
        label: "MLflow",
        icon: "flask",
        description: "ML experiment tracking, model registry, and deployment.",
        requires: ["python"],
        cliCommand: "mlflow",
        install: { kind: "pip", package: "mlflow" },
        launch: { kind: "cli", command: "mlflow", args: ["ui", "--host", "127.0.0.1"] },
        defaultPort: 5000,
        healthCheckPath: "/health",
        embedPath: "/",
        license: "Apache-2.0",
        docsUrl: "https://mlflow.org/docs/latest/",
        managedProcess: false,
    },
    {
        id: "n8n",
        label: "n8n",
        icon: "bolt",
        description: "Workflow automation with 400+ integrations.",
        requires: ["node", "npm"],
        cliCommand: "n8n",
        install: { kind: "npm", package: "n8n" },
        launch: { kind: "cli", command: "n8n", args: ["start"] },
        defaultPort: 5678,
        healthCheckPath: "/healthz",
        embedPath: "/",
        license: "Fair-code (free self-hosted)",
        docsUrl: "https://docs.n8n.io/",
        managedProcess: false,
    },
    {
        id: "grafana",
        label: "Grafana",
        icon: "chart-line",
        description: "Metrics and observability dashboards.",
        requires: [],
        cliCommand: "grafana-server",
        install: { kind: "manual" },
        launch: { kind: "none" },
        defaultPort: 3000,
        healthCheckPath: "/api/health",
        embedPath: "/",
        license: "AGPL-3.0",
        docsUrl: "https://grafana.com/docs/",
        managedProcess: false,
    },
    {
        id: "qdrant",
        label: "Qdrant",
        icon: "database",
        description: "Vector database dashboard — inspect collections, view embeddings.",
        requires: [],
        cliCommand: "qdrant",
        install: { kind: "manual" },
        launch: { kind: "none" },
        defaultPort: 6333,
        healthCheckPath: "/healthz",
        embedPath: "/dashboard",
        license: "Apache-2.0",
        docsUrl: "https://qdrant.tech/documentation/",
        managedProcess: false,
    },
    {
        id: "portainer",
        label: "Portainer",
        icon: "docker",
        description: "Docker container management UI.",
        requires: ["docker"],
        cliCommand: undefined,
        install: { kind: "manual" },
        launch: { kind: "none" },
        defaultPort: 9000,
        healthCheckPath: "/api/status",
        embedPath: "/",
        license: "Zlib",
        docsUrl: "https://docs.portainer.io/",
        managedProcess: false,
    },
];
```

---

## Phase C — Toolchain Modal UI Extension

### C1. New `ToolRow` kind and widget state

Extend the existing `ToolRow` interface in `toolchain-modal.tsx`:

```typescript
interface ToolRow {
    // ... existing fields ...
    kind: "core" | "provider" | "widget";   // add "widget"
    // widget-specific:
    widgetInstalled?: boolean;   // pip/npm package found on PATH
    widgetRunning?: boolean;     // health check passed
    widgetPort?: number;
    widgetHealthPath?: string;
    widgetLicense?: string;
    widgetRequires?: string[];   // prerequisite IDs
    prereqsMet?: boolean;        // all required core tools present
}
```

### C2. Widget section in the modal

Add a fourth section below "Agent CLIs":

```
── External Widgets ───────────────────────────────────────────────[Refresh]

  [icon] ComfyUI          [● Installed]  [● Running]          [Open Pane ↗]
         Node-based gen-AI workflows · GPL-3.0 · Port 8188
         Requires: Python ✓

  [icon] JupyterLab       [● Installed]  [○ Not running]   [Launch] [Open Pane]
         Interactive notebooks · BSD-3-Clause · Port 8888
         Requires: Python ✓

  [icon] Open WebUI       [○ Not installed]                        [Install]
         Local LLM chat UI · MIT · Port 8080
         Requires: Python ✓

  [icon] LangFlow         [○ Not installed]  Requires: Python ✗     [Install]
         Visual LLM pipelines · Apache-2.0                       (Python missing)
```

**Row states:**

| Installed | Running | Actions shown |
|-----------|---------|---------------|
| ✗ | — | `[Install]` (disabled if prereqs missing) |
| ✓ | ✗ | `[Launch]` `[Open Pane]` |
| ✓ | ✓ | `[Open Pane]` (Launch grayed out) |
| manual | — | `[Open Pane]` when running, else detect-only note |

**Prereq check:** If a widget's `requires` list contains a tool that isn't
`found` in the core tool rows, show a warning inline: "Requires Python — not
found" and disable the Install button. Clicking the tool name navigates to
the Core Tools section.

### C3. Detection logic

On modal mount (and on Refresh), run widget detection in parallel with core
tool probing:

```typescript
// For each widget:
// Step 1 — check if installed (CLI on PATH or in managed dir)
const cliResult = await RpcApi.ResolveCliCommand(TabRpcClient, {
    providerId: widget.id,
    cliCommand: widget.cliCommand ?? "",
    npmPackage: "",          // detection only, no install
    pinnedVersion: "",
});

// Step 2 — health check (is the server running?)
const healthResult = await RpcApi.WidgetHealthCheckCommand(TabRpcClient, {
    port: widget.defaultPort,
    path: widget.healthCheckPath,
    timeoutMs: 2000,
});
```

Health checks run independently from CLI detection — a server can be running
even if the CLI isn't on PATH (user may have started it via a venv, Docker, etc.).

### C4. Install flow

Clicking `[Install]` on a pip-based widget:
1. Opens the existing `AgentInstallModal` (or an inline progress expansion —
   same streaming progress UI already used for provider CLIs)
2. Calls `RpcApi.WidgetInstallCommand(TabRpcClient, { widgetId, method })`
3. Backend streams `install_chunk` events scoped to `install:<sessionId>`
4. On completion, re-probes widget row

Clicking `[Install]` on an npm-based widget reuses the existing
`install.start` handler unchanged.

Clicking `[Launch]` calls `RpcApi.WidgetLaunchCommand(TabRpcClient, { widgetId })`
then polls the health check until the server responds (max 30s, 1s intervals).
Status pill updates reactively.

Clicking `[Open Pane]` calls `createBlock({ meta: { view: widget.id } })` —
identical to clicking a widget in the action bar.

---

## Phase D — Backend: pip Install Handler

### D1. New RPC commands

Add to `agentmux-srv/src/server/`:

| Command | Handler | Purpose |
|---------|---------|---------|
| `widget.install` | `install_handlers.rs` | pip/npm install, streaming progress |
| `widget.launch` | `widget_handlers.rs` | spawn server process |
| `widget.health` | `widget_handlers.rs` | HTTP GET health check |
| `widget.list` | `widget_handlers.rs` | return catalog + detected states |

### D2. pip install handler (`widget.install`)

Mirror the existing npm install handler. Key differences:

```rust
// Determine installer: prefer uv if available, fall back to pip
let installer = if resolve_tool_path("uv").await.is_some() {
    // uv: faster, handles venvs cleanly
    ("uv", vec!["pip", "install", &package, "--target", &install_dir])
} else {
    ("pip3", vec!["install", &package, "--target", &install_dir])
};

// Install dir: ~/.agentmux/shared/widgets/<widget_id>/
// Mirrors provider pattern: ~/.agentmux/shared/providers/<name>/
```

Install into a managed directory (not system Python) so:
- No `sudo` required
- Doesn't pollute the user's system Python
- Easy to uninstall (delete the directory)
- AgentMux controls the exact version

Binary resolution after install:
```rust
// Check ~/.agentmux/shared/widgets/<id>/bin/<cliCommand>
// or ~/.agentmux/shared/widgets/<id>/<cliCommand>
// (pip --target layout differs from npm --prefix layout)
```

### D3. Health check handler (`widget.health`)

New lightweight RPC — does not need streaming, just a single request/response:

```rust
pub struct WidgetHealthCheckRequest {
    pub port: u16,
    pub path: String,
    pub timeout_ms: u64,
}

pub struct WidgetHealthCheckResult {
    pub running: bool,
    pub status_code: Option<u16>,
    pub latency_ms: Option<u64>,
}

// Implementation:
async fn check_widget_health(req: WidgetHealthCheckRequest) -> WidgetHealthCheckResult {
    let url = format!("http://127.0.0.1:{}{}", req.port, req.path);
    let timeout = Duration::from_millis(req.timeout_ms.min(5000));
    match reqwest::Client::new()
        .get(&url)
        .timeout(timeout)
        .send()
        .await
    {
        Ok(resp) => WidgetHealthCheckResult {
            running: resp.status().is_success(),
            status_code: Some(resp.status().as_u16()),
            latency_ms: Some(elapsed_ms),
        },
        Err(_) => WidgetHealthCheckResult { running: false, .. },
    }
}
```

### D4. Launch handler (`widget.launch`) — MVP deferred

For the MVP, `managedProcess: false` for all widgets — users launch externally
and AgentMux detects. The `widget.launch` handler is designed but deferred to
a follow-up sprint.

When implemented: spawn using the existing `SubprocessSpawnConfig` pattern from
`blockcontroller/subprocess.rs`. Register PID in `process_tracker` so it
surfaces in the Swarm pane. Kill on AgentMux exit via the Job Object (Windows)
or process group signal (Unix).

---

## Phase E — View Types for Widget Panes

Each external widget needs a ViewModel that:
1. Calls `widget.health` on mount to determine initial state
2. Renders either the "server not running" placeholder or the embedded
   `<iframe>` / `WebContentsView` pointing at `http://127.0.0.1:<port><embedPath>`
3. Polls health every 10s to detect server start/stop reactively

Since all widgets follow the same embed pattern, a **shared
`ExternalWidgetViewModel`** class can be parameterized by `ExternalWidget`:

```typescript
export class ExternalWidgetViewModel implements ViewModel {
    viewType: string;
    viewComponent = ExternalWidgetView;

    constructor(blockId: string, nodeModel: BlockNodeModel, widget: ExternalWidget) {
        this.viewType = widget.id;
        this.widget = widget;
        this.blockId = blockId;
        this.viewName = () => widget.label;
        this.viewIcon = () => widget.icon;
        // health poll
        this.pollHealth();
    }

    private async pollHealth() {
        const result = await RpcApi.WidgetHealthCheckCommand(...);
        this.setRunning(result.running);
        // re-poll every 10s
        this.pollTimer = setTimeout(() => this.pollHealth(), 10_000);
    }
}
```

Register in `block-registry.ts`:
```typescript
for (const widget of EXTERNAL_WIDGETS) {
    registerViewType(widget.id, (blockId, nodeModel) =>
        new ExternalWidgetViewModel(blockId, nodeModel, widget)
    );
}
```

This is also the **Phase 1 registry refactor** from the pluggable widget
feasibility report — external widgets are the first natural use case for
`registerViewType()`.

---

## Implementation Plan

| Phase | Work | Effort | Prerequisite |
|-------|------|--------|-------------|
| **A** | Add Python + uv to `CORE_TOOLS` | 2h | none |
| **B** | Create `widget-catalog.ts` with 10 widgets | 2h | none |
| **C** | Toolchain modal: External Widgets section (detection + status only) | 1d | A + B |
| **D1** | `widget.health` RPC handler | 3h | none |
| **D2** | `widget.install` pip handler | 1d | none |
| **D3** | Wire Install + Launch buttons in modal | 4h | C + D1 + D2 |
| **E** | `ExternalWidgetViewModel` + `ExternalWidgetView` + registry | 1d | B + D1 |
| **E2** | `registerViewType()` refactor in `block-registry.ts` | 3h | none |
| **Total** | | **~4.5 days** | |

Phases A + B + C + D1 can ship as a cohesive "detection + status" release
(read-only, same tier as current P1) in ~2 days. Install and pane-open in the
second pass.

---

## Out of Scope

- Managed process lifecycle (`managedProcess: true`) — deferred to a follow-up
- Widget update/upgrade flow — deferred
- Widget uninstall UI — deferred (users can delete `~/.agentmux/shared/widgets/<id>/`)
- Third-party/community widget catalog — deferred (requires plugin API from
  `RESEARCH_PLUGGABLE_WIDGET_API_2026_06_21.md`)
- Authentication/token passing into embedded UIs (JupyterLab token, etc.) — P2
- Comfy Desktop integration (auto-detect existing Comfy Desktop install) — P2
