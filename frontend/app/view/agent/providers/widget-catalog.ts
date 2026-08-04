// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * External widget catalog — local-server tools AgentMux can detect, install,
 * and embed as panes. See docs/specs/SPEC_TOOLCHAIN_MANAGER_EXTERNAL_WIDGETS_2026_06_22.md.
 *
 * Detection reuses `resolvecli` RPC (CLI on PATH).
 * Health check uses `widget.health` RPC (HTTP GET on the default port).
 * Install uses `widget.install` RPC (pip or npm, streamed).
 * Open Pane calls createBlock({ meta: { view: widget.id } }).
 */

import type { Platform } from "./toolchain-catalog";

/** How the widget is installed onto the user's machine. */
type InstallMethod =
    | { kind: "pip"; package: string; extraArgs?: string[] }
    | { kind: "npm"; package: string; version?: string }
    | { kind: "manual" }; // user installs themselves; we only detect + health-check

export interface ExternalWidget {
    /** Stable ID — also the view type string for the pane. */
    id: string;
    label: string;
    /** fa-solid fa-<icon> */
    icon: string;
    description: string;
    /**
     * CoreTool IDs that must be present before install is enabled.
     * Empty array = no prerequisites (manual-install tools like Grafana/Qdrant).
     */
    requires: string[];
    /** CLI binary name to detect on PATH (or in managed install dir). */
    cliCommand?: string;
    /** Per-platform CLI command override. */
    cliCommandByPlatform?: Partial<Record<Platform, string>>;
    install: InstallMethod;
    /** Default port the server listens on. */
    defaultPort: number;
    /** URL path to GET for liveness check — must return 2xx when healthy. */
    healthCheckPath: string;
    /**
     * If set, the response body must contain this substring for the health
     * check to pass. Used to distinguish services that share a default port
     * (e.g. Flowise and Grafana both default to 3000).
     */
    healthCheckBodyContains?: string;
    /** URL path to embed (relative to http://127.0.0.1:<port>). */
    embedPath: string;
    license: string;
    docsUrl: string;
}

/** Resolve the CLI command for the current platform. */
export function widgetCliCommandForPlatform(
    widget: ExternalWidget,
    plat: Platform
): string | undefined {
    return widget.cliCommandByPlatform?.[plat] ?? widget.cliCommand;
}

export const EXTERNAL_WIDGETS: ExternalWidget[] = [
    {
        id: "comfyui",
        label: "ComfyUI",
        icon: "diagram-project",
        description: "Node-based generative AI workflow editor. Stable Diffusion, Flux, video, audio.",
        requires: ["python"],
        cliCommand: "comfyui",
        cliCommandByPlatform: { windows: "comfyui" },
        install: { kind: "pip", package: "comfyui" },
        defaultPort: 8188,
        healthCheckPath: "/system_stats",
        healthCheckBodyContains: "devices",
        embedPath: "/",
        license: "GPL-3.0",
        docsUrl: "https://docs.comfy.org/",
    },
    {
        id: "jupyterlab",
        label: "JupyterLab",
        icon: "book-open",
        description: "Interactive Python notebook environment for data science and ML.",
        requires: ["python"],
        cliCommand: "jupyter",
        install: { kind: "pip", package: "jupyterlab" },
        defaultPort: 8888,
        healthCheckPath: "/api",
        healthCheckBodyContains: "version",
        embedPath: "/",
        license: "BSD-3-Clause",
        docsUrl: "https://jupyterlab.readthedocs.io/",
    },
    {
        id: "open-webui",
        label: "Open WebUI",
        icon: "comments",
        description: "Local chat UI for Ollama and OpenAI-compatible models.",
        requires: ["python"],
        cliCommand: "open-webui",
        install: { kind: "pip", package: "open-webui" },
        defaultPort: 8080,
        healthCheckPath: "/health",
        healthCheckBodyContains: "status",
        embedPath: "/",
        license: "MIT",
        docsUrl: "https://docs.openwebui.com/",
    },
    {
        id: "langflow",
        label: "LangFlow",
        icon: "code-branch",
        description: "Visual drag-and-drop LLM pipeline builder.",
        requires: ["python"],
        cliCommand: "langflow",
        install: { kind: "pip", package: "langflow" },
        defaultPort: 7860,
        healthCheckPath: "/health",
        healthCheckBodyContains: "alive",
        embedPath: "/",
        license: "Apache-2.0",
        docsUrl: "https://docs.langflow.org/",
    },
    {
        id: "flowise",
        label: "Flowise",
        icon: "sitemap",
        description: "Visual LangChain/LlamaIndex pipeline builder.",
        requires: ["node", "npm"],
        cliCommand: "flowise",
        cliCommandByPlatform: { windows: "flowise.cmd" },
        install: { kind: "npm", package: "flowise" },
        defaultPort: 3000,
        healthCheckPath: "/api/v1/ping",
        healthCheckBodyContains: "pong",
        embedPath: "/",
        license: "Apache-2.0",
        docsUrl: "https://docs.flowiseai.com/",
    },
    {
        id: "mlflow",
        label: "MLflow",
        icon: "flask",
        description: "ML experiment tracking, model registry, and deployment.",
        requires: ["python"],
        cliCommand: "mlflow",
        install: { kind: "pip", package: "mlflow" },
        defaultPort: 5000,
        healthCheckPath: "/health",
        healthCheckBodyContains: "Healthy",
        embedPath: "/",
        license: "Apache-2.0",
        docsUrl: "https://mlflow.org/docs/latest/",
    },
    {
        id: "n8n",
        label: "n8n",
        icon: "bolt",
        description: "Workflow automation with 400+ integrations. Self-hosted.",
        requires: ["node", "npm"],
        cliCommand: "n8n",
        cliCommandByPlatform: { windows: "n8n.cmd" },
        install: { kind: "npm", package: "n8n" },
        defaultPort: 5678,
        healthCheckPath: "/healthz",
        healthCheckBodyContains: "status",
        embedPath: "/",
        license: "Fair-code (free self-hosted)",
        docsUrl: "https://docs.n8n.io/",
    },
    {
        id: "grafana",
        label: "Grafana",
        icon: "chart-line",
        description: "Metrics and observability dashboards.",
        requires: [],
        cliCommand: "grafana-server",
        cliCommandByPlatform: { windows: "grafana-server.exe" },
        install: { kind: "manual" },
        defaultPort: 3000,
        healthCheckPath: "/api/health",
        healthCheckBodyContains: "database",
        embedPath: "/",
        license: "AGPL-3.0",
        docsUrl: "https://grafana.com/docs/grafana/latest/setup-grafana/installation/",
    },
    {
        id: "qdrant",
        label: "Qdrant",
        icon: "database",
        description: "Vector database — inspect collections, browse embeddings.",
        requires: [],
        cliCommand: "qdrant",
        cliCommandByPlatform: { windows: "qdrant.exe" },
        install: { kind: "manual" },
        defaultPort: 6333,
        healthCheckPath: "/healthz",
        healthCheckBodyContains: "qdrant",
        embedPath: "/dashboard",
        license: "Apache-2.0",
        docsUrl: "https://qdrant.tech/documentation/guides/installation/",
    },
    {
        id: "portainer",
        label: "Portainer",
        icon: "ship",
        description: "Docker container management UI.",
        requires: ["docker"],
        cliCommand: undefined,
        install: { kind: "manual" },
        defaultPort: 9000,
        healthCheckPath: "/api/status",
        healthCheckBodyContains: "InstanceID",
        embedPath: "/",
        license: "Zlib",
        docsUrl: "https://docs.portainer.io/start/install-ce/server/docker/linux",
    },
];
