// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

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
 */

export type Platform = "windows" | "macos" | "linux";

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
    /** Font Awesome (solid) icon name, rendered as `fa-solid fa-<icon>`. */
    icon: string;
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
}

/** Resolve the CLI command for the current platform. */
export function cliCommandForPlatform(tool: CoreTool, plat: Platform): string {
    return tool.cliCommandByPlatform?.[plat] ?? tool.cliCommand;
}

const NODE_DOWNLOAD = "https://nodejs.org/en/download";

export const CORE_TOOLS: CoreTool[] = [
    {
        id: "node",
        cliCommand: "node",
        label: "Node.js",
        icon: "cube",
        minVersion: "18",
        description: "JavaScript runtime — required to install & run the npm-based agent CLIs.",
        docsUrl: "https://nodejs.org/",
        installUrls: { windows: NODE_DOWNLOAD, macos: NODE_DOWNLOAD, linux: NODE_DOWNLOAD },
        installCommand: { macos: "brew install node", linux: "sudo apt install -y nodejs npm" },
        brewFormula: "node",
    },
    {
        id: "npm",
        cliCommand: "npm",
        label: "npm",
        icon: "box",
        description: "Node package manager — ships with Node.js; installs the agent CLIs.",
        docsUrl: "https://docs.npmjs.com/",
        installUrls: { windows: NODE_DOWNLOAD, macos: NODE_DOWNLOAD, linux: NODE_DOWNLOAD },
        installCommand: { macos: "brew install node", linux: "sudo apt install -y nodejs npm" },
        brewFormula: "node",
    },
    {
        id: "git",
        cliCommand: "git",
        label: "Git",
        icon: "code-branch",
        minVersion: "2.23",
        description: "Version control — used by Claude/OpenClaw for project context.",
        docsUrl: "https://git-scm.com/",
        installUrls: {
            windows: "https://git-scm.com/download/win",
            macos: "https://git-scm.com/download/mac",
            linux: "https://git-scm.com/download/linux",
        },
        installCommand: { macos: "brew install git", linux: "sudo apt install -y git" },
        brewFormula: "git",
    },
    {
        id: "docker",
        cliCommand: "docker",
        label: "Docker",
        icon: "box-open",
        optional: true,
        description: "Container runtime — only needed for container-mode agents.",
        docsUrl: "https://docs.docker.com/get-docker/",
        installUrls: {
            windows: "https://docs.docker.com/desktop/install/windows-install/",
            macos: "https://docs.docker.com/desktop/install/mac-install/",
            linux: "https://docs.docker.com/engine/install/",
        },
        brewFormula: "docker",
    },
    {
        id: "python",
        cliCommand: "python3",
        cliCommandByPlatform: { windows: "python" },
        label: "Python",
        icon: "snake",
        minVersion: "3.10",
        description: "Required runtime for ComfyUI, JupyterLab, MLflow, and other AI tools.",
        docsUrl: "https://www.python.org/downloads/",
        installUrls: {
            windows: "https://www.python.org/downloads/windows/",
            macos: "https://www.python.org/downloads/macos/",
            linux: "https://www.python.org/downloads/source/",
        },
        installCommand: {
            windows: "winget install Python.Python.3.12",
            macos: "brew install python@3.12",
            linux: "sudo apt install -y python3 python3-pip python3-venv",
        },
        brewFormula: "python@3.12",
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
