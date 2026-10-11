// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getPlatform } from "@/util/platformutil";

/**
 * Core toolchain catalog — the system tools AgentMux relies on beyond the
 * provider CLIs: Node.js, npm, Git, and Docker. The Toolchain modal renders
 * one row per entry (detected version + path + status) alongside the provider
 * CLIs from `PROVIDERS`. This is the one place to add "anything we need" later
 * (ripgrep, uv/python for kimi, …).
 *
 * Detection reuses the existing `resolvecli` RPC (versioned-install-dir →
 * system PATH → `--version`), so a core tool with an empty `npmPackage` simply
 * resolves on PATH. See docs/specs/SPEC_TOOLCHAIN_MANAGER_2026-06-15.md §5.
 *
 * `checkKind` distinguishes what "installed" actually means for a tool:
 * `"path"` (the default) means the binary resolves on PATH — correct for
 * static tools (git, node, python) that have no separate running-or-not
 * state. `"liveness"` means the binary being on PATH is NOT sufficient —
 * the tool is backed by a daemon/service that can be installed but not
 * running (Docker being the motivating case: `docker --version` succeeds
 * even when Docker Desktop is stopped). See `frontend/app/store/
 * toolchain-capabilities.ts` — the single point of entry that dispatches
 * to the right backend check based on this field, instead of every
 * consumer deciding for itself which check answers "is it available."
 * docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md has the
 * incident this field exists to prevent from recurring for the next
 * daemon-backed tool.
 */

export type Platform = "windows" | "macos" | "linux";
type CheckKind = "path" | "liveness";

export interface CoreTool {
    /** Stable id. */
    id: string;
    /**
     * Binary name probed on PATH. Use `cliCommandByPlatform` to override on
     * specific platforms (e.g. python3 on Unix, python on Windows).
     */
    cliCommand: string;
    /** Per-platform CLI command override — takes precedence over `cliCommand`. */
    cliCommandByPlatform?: Partial<Record<Platform, string>>;
    label: string;
    /** Font Awesome (solid) icon name, rendered as `fa-solid fa-<icon>`.
     *  Always required — the guaranteed fallback for tools with no brand
     *  glyph (see `brandIcon`). */
    icon: string;
    /** Font Awesome *brands* icon name (rendered as `fa-brands fa-<icon>`),
     *  preferred over `icon` when present — see `rowIconClass`. Omit for a
     *  tool with no official mark in Font Awesome's bundled brand set
     *  (e.g. `uv`); it then falls back to `icon` as before.
     *  SPEC_SYSTEM_TOOL_INSTALL_DETAILS_AUTOSCROLL_2026_09_10.md §6. */
    brandIcon?: string;
    /** Recommended minimum version (warn-only — never blocks). */
    minVersion?: string;
    /** Optional — a missing optional tool shows an info pill, not a warning. */
    optional?: boolean;
    description?: string;
    docsUrl?: string;
    /** Official install landing page per platform. */
    installUrls: Record<Platform, string>;
    /** Copyable one-liner per platform (shown next to the install link). */
    installCommand?: Partial<Record<Platform, string>>;
    /** Homebrew formula — enables the P3 one-click install when brew exists. */
    brewFormula?: string;
    /** winget package identifier (e.g. "Git.Git") — mirrors `brewFormula`'s
     *  role on Windows. Display-only, like `brewFormula`: the backend's own
     *  fixed catalog (`system_install_handlers.rs`) is the execution-
     *  authoritative copy — this field is never sent to the backend, only
     *  used to decide whether to show the one-click Install action. See
     *  docs/specs/SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md. */
    wingetId?: string;
    /** What "available" means for this tool. Defaults to `"path"` when omitted. */
    checkKind?: CheckKind;
}

/** Resolve the CLI command for the current platform. */
export function cliCommandForPlatform(tool: CoreTool, plat: Platform): string {
    return tool.cliCommandByPlatform?.[plat] ?? tool.cliCommand;
}

/**
 * Font Awesome class for a tool's row icon — prefers the real brand mark
 * (`fa-brands fa-<brandIcon>`) when one exists, falling back to the
 * generic solid glyph (`fa-solid fa-<icon>`) otherwise (e.g. `uv`, which
 * has no icon in Font Awesome's bundled brand set).
 * SPEC_SYSTEM_TOOL_INSTALL_DETAILS_AUTOSCROLL_2026_09_10.md §6.
 */
export function rowIconClass(icon: string, brandIcon?: string): string {
    return brandIcon ? `fa-brands fa-${brandIcon}` : `fa-solid fa-${icon}`;
}

/**
 * The current OS as a `Platform`. Single implementation — was previously
 * duplicated as a local `platformKey()` in toolchain-view.tsx.
 */
export function currentPlatform(): Platform {
    switch (getPlatform()) {
        case "win32": return "windows";
        case "darwin": return "macos";
        default: return "linux";
    }
}

const NODE_DOWNLOAD = "https://nodejs.org/en/download";

export const CORE_TOOLS: CoreTool[] = [
    {
        id: "node",
        cliCommand: "node",
        label: "Node.js",
        icon: "cube",
        brandIcon: "node-js",
        minVersion: "18",
        description: "JavaScript runtime — required to install & run the npm-based agent CLIs.",
        docsUrl: "https://nodejs.org/",
        installUrls: { windows: NODE_DOWNLOAD, macos: NODE_DOWNLOAD, linux: NODE_DOWNLOAD },
        installCommand: {
            windows: "winget install --id OpenJS.NodeJS.LTS -e",
            macos: "brew install node",
            linux: "sudo apt install -y nodejs npm",
        },
        brewFormula: "node",
        wingetId: "OpenJS.NodeJS.LTS",
    },
    {
        id: "npm",
        cliCommand: "npm",
        label: "npm",
        icon: "box",
        brandIcon: "npm",
        description: "Node package manager — ships with Node.js; installs the agent CLIs.",
        docsUrl: "https://docs.npmjs.com/",
        installUrls: { windows: NODE_DOWNLOAD, macos: NODE_DOWNLOAD, linux: NODE_DOWNLOAD },
        // Same target as "node" above — npm ships bundled with Node, there
        // is no separate winget/brew/apt package for it. The one-click
        // Install action for this row re-triggers the same backend
        // resolution as node's (both resolve to the same tool_id-keyed
        // catalog entry server-side); modeled as two rows here only
        // because the Toolchain modal displays them as two detectable
        // binaries, not because they install separately.
        installCommand: {
            windows: "winget install --id OpenJS.NodeJS.LTS -e",
            macos: "brew install node",
            linux: "sudo apt install -y nodejs npm",
        },
        brewFormula: "node",
        wingetId: "OpenJS.NodeJS.LTS",
    },
    {
        id: "git",
        cliCommand: "git",
        label: "Git",
        icon: "code-branch",
        brandIcon: "git-alt",
        minVersion: "2.23",
        description: "Version control — used by Claude/OpenClaw for project context.",
        docsUrl: "https://git-scm.com/",
        installUrls: {
            windows: "https://git-scm.com/download/win",
            macos: "https://git-scm.com/download/mac",
            linux: "https://git-scm.com/download/linux",
        },
        installCommand: {
            windows: "winget install --id Git.Git -e",
            macos: "brew install git",
            linux: "sudo apt install -y git",
        },
        brewFormula: "git",
        wingetId: "Git.Git",
    },
    {
        id: "docker",
        cliCommand: "docker",
        label: "Docker",
        icon: "box-open",
        brandIcon: "docker",
        optional: true,
        description: "Container runtime — only needed for container-mode agents.",
        docsUrl: "https://docs.docker.com/get-docker/",
        installUrls: {
            windows: "https://docs.docker.com/desktop/install/windows-install/",
            macos: "https://docs.docker.com/desktop/install/mac-install/",
            linux: "https://docs.docker.com/engine/install/",
        },
        brewFormula: "docker",
        // The CLI binary being on PATH doesn't mean the daemon is running
        // (Docker Desktop can be installed but stopped) — this tool needs
        // a liveness check, not a path check. See the `checkKind` doc
        // comment above.
        checkKind: "liveness",
    },
    {
        id: "python",
        cliCommand: "python3",
        cliCommandByPlatform: { windows: "python" },
        label: "Python",
        icon: "code",
        brandIcon: "python",
        minVersion: "3.10",
        description: "Required runtime for ComfyUI, JupyterLab, MLflow, and other AI tools.",
        docsUrl: "https://www.python.org/downloads/",
        installUrls: {
            windows: "https://www.python.org/downloads/windows/",
            macos: "https://www.python.org/downloads/macos/",
            linux: "https://www.python.org/downloads/source/",
        },
        installCommand: {
            windows: "winget install --id Python.Python.3.12 -e",
            macos: "brew install python@3.12",
            linux: "sudo apt install -y python3 python3-pip python3-venv",
        },
        brewFormula: "python@3.12",
        wingetId: "Python.Python.3.12",
    },
    {
        id: "uv",
        cliCommand: "uv",
        label: "uv",
        icon: "bolt",
        optional: true,
        description: "Fast Python package manager — 10–100× faster than pip. Recommended for widget installs.",
        docsUrl: "https://docs.astral.sh/uv/",
        installUrls: {
            windows: "https://docs.astral.sh/uv/getting-started/installation/",
            macos: "https://docs.astral.sh/uv/getting-started/installation/",
            linux: "https://docs.astral.sh/uv/getting-started/installation/",
        },
        installCommand: {
            windows: 'powershell -c "irm https://astral.sh/uv/install.ps1 | iex"',
            macos: "brew install uv",
            linux: "curl -LsSf https://astral.sh/uv/install.sh | sh",
        },
        brewFormula: "uv",
    },
];

/**
 * Local-model toolchain — inference runtimes and local-capable agent CLIs,
 * rendered in the Toolchain pane's own "Local models" section. Detection
 * only (PATH probe via `resolvecli`): none of these are in the backend's
 * system-install catalog, so each row offers the copyable command and the
 * install link, never the one-click install. Kept out of `CORE_TOOLS` so
 * `toolchain-capabilities.ts` and the one-click installer never treat them
 * as tools AgentMux itself needs.
 *
 * The agent entries (OpenCode, Goose, Crush, Aider) are NOT launchable as
 * AgentMux panes yet — they are listed so users can see what's installed.
 */
export const LOCAL_MODEL_TOOLS: CoreTool[] = [
    {
        id: "ollama",
        cliCommand: "ollama",
        label: "Ollama",
        icon: "server",
        optional: true,
        description: "Local model runtime — serves OpenAI- and Anthropic-compatible APIs on :11434.",
        docsUrl: "https://docs.ollama.com/",
        installUrls: {
            windows: "https://ollama.com/download/windows",
            macos: "https://ollama.com/download/mac",
            linux: "https://docs.ollama.com/linux",
        },
        installCommand: {
            windows: "winget install --id Ollama.Ollama -e",
            macos: "brew install ollama",
            linux: "curl -fsSL https://ollama.com/install.sh | sh",
        },
    },
    {
        id: "llama-cpp",
        cliCommand: "llama-server",
        label: "llama.cpp",
        icon: "microchip",
        optional: true,
        description: "Inference engine — llama-server serves GGUF models over OpenAI- and Anthropic-compatible APIs.",
        docsUrl: "https://github.com/ggml-org/llama.cpp/blob/master/docs/install.md",
        installUrls: {
            windows: "https://github.com/ggml-org/llama.cpp/releases",
            macos: "https://github.com/ggml-org/llama.cpp/blob/master/docs/install.md",
            linux: "https://github.com/ggml-org/llama.cpp/releases",
        },
        installCommand: {
            windows: "winget install llama.cpp",
            macos: "brew install llama.cpp",
            linux: "brew install llama.cpp",
        },
    },
    {
        id: "lmstudio",
        cliCommand: "lms",
        label: "LM Studio",
        icon: "desktop",
        optional: true,
        description: "Local model runtime (desktop app or headless daemon) — serves OpenAI, Responses, and Anthropic APIs on :1234.",
        docsUrl: "https://lmstudio.ai/docs/developer/core/headless",
        installUrls: {
            windows: "https://lmstudio.ai/download",
            macos: "https://lmstudio.ai/download",
            linux: "https://lmstudio.ai/download",
        },
        installCommand: {
            windows: "irm https://lmstudio.ai/install.ps1 | iex",
            macos: "curl -fsSL https://lmstudio.ai/install.sh | bash",
            linux: "curl -fsSL https://lmstudio.ai/install.sh | bash",
        },
    },
    {
        id: "llmfit",
        cliCommand: "llmfit",
        label: "llmfit",
        icon: "gauge-high",
        optional: true,
        description: "Hardware check — recommends which local models fit this machine's GPU/RAM.",
        docsUrl: "https://github.com/AlexsJones/llmfit",
        installUrls: {
            windows: "https://github.com/AlexsJones/llmfit#installation",
            macos: "https://github.com/AlexsJones/llmfit#installation",
            linux: "https://github.com/AlexsJones/llmfit#installation",
        },
        installCommand: {
            windows: "scoop install llmfit",
            macos: "brew install llmfit",
            linux: "curl -fsSL https://llmfit.axjns.dev/install.sh | sh",
        },
    },
    {
        id: "opencode",
        cliCommand: "opencode",
        label: "OpenCode",
        icon: "terminal",
        optional: true,
        description: "Open-source coding agent with built-in Ollama / LM Studio / llama.cpp support. Not yet launchable as an AgentMux pane.",
        docsUrl: "https://opencode.ai/docs/",
        installUrls: {
            windows: "https://opencode.ai/docs/",
            macos: "https://opencode.ai/docs/",
            linux: "https://opencode.ai/docs/",
        },
        installCommand: {
            windows: "npm install -g opencode-ai",
            macos: "brew install anomalyco/tap/opencode",
            linux: "curl -fsSL https://opencode.ai/install | bash",
        },
    },
    {
        id: "goose",
        cliCommand: "goose",
        label: "Goose",
        icon: "feather",
        optional: true,
        description: "Open-source coding agent (Agentic AI Foundation) with native Ollama support. Not yet launchable as an AgentMux pane.",
        docsUrl: "https://goose-docs.ai/docs/getting-started/installation/",
        installUrls: {
            windows: "https://goose-docs.ai/docs/getting-started/installation/",
            macos: "https://goose-docs.ai/docs/getting-started/installation/",
            linux: "https://goose-docs.ai/docs/getting-started/installation/",
        },
        installCommand: {
            macos: "brew install block-goose-cli",
            linux: "curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh | bash",
        },
    },
    {
        id: "crush",
        cliCommand: "crush",
        label: "Crush",
        icon: "wand-magic-sparkles",
        optional: true,
        description: "Terminal coding agent with Ollama / LM Studio / llama.cpp provider types. Not yet launchable as an AgentMux pane.",
        docsUrl: "https://github.com/charmbracelet/crush",
        installUrls: {
            windows: "https://github.com/charmbracelet/crush#installation",
            macos: "https://github.com/charmbracelet/crush#installation",
            linux: "https://github.com/charmbracelet/crush#installation",
        },
        installCommand: {
            windows: "winget install charmbracelet.crush",
            macos: "brew install charmbracelet/tap/crush",
            linux: "npm install -g @charmland/crush",
        },
    },
    {
        id: "aider",
        cliCommand: "aider",
        label: "Aider",
        icon: "user-pen",
        optional: true,
        description: "Terminal pair programmer; runs local models via Ollama. Not yet launchable as an AgentMux pane.",
        docsUrl: "https://aider.chat/docs/install.html",
        installUrls: {
            windows: "https://aider.chat/docs/install.html",
            macos: "https://aider.chat/docs/install.html",
            linux: "https://aider.chat/docs/install.html",
        },
        installCommand: {
            windows: 'powershell -ExecutionPolicy ByPass -c "irm https://aider.chat/install.ps1 | iex"',
            macos: "curl -LsSf https://aider.chat/install.sh | sh",
            linux: "curl -LsSf https://aider.chat/install.sh | sh",
        },
    },
];
