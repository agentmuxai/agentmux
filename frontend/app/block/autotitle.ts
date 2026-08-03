// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Auto-title generation utilities for pane labels
 * Generates contextual titles based on block type and content
 */

import { isBlank } from "@/util/util";

/**
 * Environment variable name for agent identity
 */
const AGENT_ENV_VAR = "AGENTMUX_AGENT_ID" as const;

/**
 * Environment variable name for agent color (background)
 */
const AGENT_COLOR_ENV_VAR = "AGENTMUX_AGENT_COLOR" as const;

/**
 * Environment variable name for agent text color
 */
const AGENT_TEXT_COLOR_ENV_VAR = "AGENTMUX_AGENT_TEXT_COLOR" as const;

/**
 * Default colors for known agents (used when no color env var is set)
 */
const DEFAULT_AGENT_COLORS: Record<string, string> = {
    AgentA: "#1e3a5f",  // Dark blue
    AgentX: "#ef4444",  // Red (matches claw assignment)
    AgentY: "#eab308",  // Yellow/Gold
    AgentG: "#f59e0b",  // Amber
    Agent1: "#3b82f6",  // Blue
    Agent2: "#06b6d4",  // Cyan
    Agent3: "#ec4899",  // Pink
    Agent4: "#ef4444",  // Red
    Agent5: "#84cc16",  // Lime
};

/**
 * Default text colors for known agents (used when no text color env var is set)
 * These are optimized for readability against the default background colors
 */
const DEFAULT_AGENT_TEXT_COLORS: Record<string, string> = {
    AgentA: "#ffffff",  // White on dark blue
    AgentX: "#ffffff",  // White on red
    AgentY: "#000000",  // Black on yellow/gold
    AgentG: "#000000",  // Black on amber
    Agent1: "#ffffff",  // White on blue
    Agent2: "#000000",  // Black on cyan
    Agent3: "#ffffff",  // White on pink
    Agent4: "#ffffff",  // White on red
    Agent5: "#000000",  // Black on lime
};

/**
 * Detect agent color from environment variable or use default
 */
export function detectAgentColor(envVars: Record<string, string> | undefined, agentId: string | null): string | null {
    // Check env var
    if (envVars) {
        const value = envVars[AGENT_COLOR_ENV_VAR];
        if (!isBlank(value)) {
            return value!.trim();
        }
    }

    // Fall back to default color for known agents
    if (agentId && DEFAULT_AGENT_COLORS[agentId]) {
        return DEFAULT_AGENT_COLORS[agentId];
    }

    return null;
}

/**
 * Detect agent text color from environment variable or use default
 * Returns the text color to use in the pane header for optimal readability
 */
export function detectAgentTextColor(envVars: Record<string, string> | undefined, agentId: string | null): string | null {
    // Check env var first
    if (envVars) {
        const value = envVars[AGENT_TEXT_COLOR_ENV_VAR];
        if (!isBlank(value)) {
            return value!.trim();
        }
    }

    // Fall back to default text color for known agents
    if (agentId && DEFAULT_AGENT_TEXT_COLORS[agentId]) {
        return DEFAULT_AGENT_TEXT_COLORS[agentId];
    }

    return null;
}

/**
 * Detect agent identity from environment variable in block metadata
 *
 * "Terminal" is rejected: it is the default title for plain terminal panes,
 * not an agent identity. Shell profiles have been observed exporting
 * AGENTMUX_AGENT_ID="Terminal" (+ AGENTMUX_AGENT_COLOR="#000000") as a
 * "label my plain panes" hack, which turned every plain terminal into a
 * pseudo-agent and painted its focused border black. See
 * reports/ANALYSIS_WIN11_PANE_BORDER_HIGHLIGHT_2026-06-12 (agenty workspace).
 */
export function detectAgentFromEnv(envVars: Record<string, string> | undefined): string | null {
    if (!envVars) {
        return null;
    }

    const value = envVars[AGENT_ENV_VAR];
    if (!isBlank(value)) {
        const trimmed = value!.trim();
        if (trimmed === "" || trimmed.toLowerCase() === "terminal") {
            return null;
        }
        return trimmed;
    }

    return null;
}

/**
 * Minimum WCAG relative luminance for an agent color to be used as the
 * focused pane ring. Rejects degenerate values (#000, near-black) that would
 * make the focused border invisible against the dark pane chrome — those
 * panes fall back to the theme accent ring instead. Deliberately low so
 * dark-but-visible agent palettes (e.g. AgentA's #1e3a5f, L≈0.041) pass.
 */
const MIN_FOCUS_RING_LUMINANCE = 0.03;

interface ParsedRgba {
    r: number; // 0-255
    g: number;
    b: number;
    a: number; // 0-1
}

/**
 * Parse a CSS color in #rgb/#rgba/#rrggbb/#rrggbbaa or rgb()/rgba() form.
 * Named colors and other syntaxes return null (callers treat that as
 * "not usable", falling back to the theme accent).
 */
function parseCssColor(color: string | null | undefined): ParsedRgba | null {
    if (isBlank(color)) {
        return null;
    }
    const c = color!.trim().toLowerCase();

    const hexMatch = c.match(/^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/);
    if (hexMatch) {
        let hex = hexMatch[1];
        if (hex.length <= 4) {
            hex = hex.split("").map((ch) => ch + ch).join("");
        }
        const r = parseInt(hex.slice(0, 2), 16);
        const g = parseInt(hex.slice(2, 4), 16);
        const b = parseInt(hex.slice(4, 6), 16);
        const a = hex.length === 8 ? parseInt(hex.slice(6, 8), 16) / 255 : 1;
        return { r, g, b, a };
    }

    const fnMatch = c.match(/^rgba?\(\s*([\d.]+)\s*[, ]\s*([\d.]+)\s*[, ]\s*([\d.]+)\s*(?:[,/]\s*([\d.]+%?)\s*)?\)$/);
    if (fnMatch) {
        const r = Number(fnMatch[1]);
        const g = Number(fnMatch[2]);
        const b = Number(fnMatch[3]);
        let a = 1;
        if (fnMatch[4] != null) {
            a = fnMatch[4].endsWith("%") ? Number(fnMatch[4].slice(0, -1)) / 100 : Number(fnMatch[4]);
        }
        if ([r, g, b, a].some((n) => Number.isNaN(n))) {
            return null;
        }
        return { r, g, b, a };
    }

    return null;
}

/** WCAG 2.x relative luminance (0 = black, 1 = white). */
function relativeLuminance({ r, g, b }: ParsedRgba): number {
    const lin = (v: number) => {
        const s = v / 255;
        return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    };
    return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/**
 * Whether an agent color is usable as the focused pane ring.
 *
 * The focused border is the selection affordance — an identity color may tint
 * it, but must never erase it. Rejects unparseable colors, (near-)transparent
 * colors, and near-black colors whose ring would be invisible on the dark
 * pane chrome. Callers fall back to the theme accent when this returns false.
 */
export function isUsableFocusRingColor(color: string | null | undefined): boolean {
    const rgba = parseCssColor(color);
    if (!rgba) {
        return false;
    }
    if (rgba.a < 0.5) {
        return false;
    }
    return relativeLuminance(rgba) >= MIN_FOCUS_RING_LUMINANCE;
}

/**
 * Detect agent identity from explicit agent-workspaces directory pattern only
 * Looks for patterns like /agent-workspaces/agent2/ or C:\Code\agent-workspaces\agent3\
 * This is an intentional opt-in structure that works for all connection types.
 * Returns the agent ID (e.g., "Agent2", "AgentX") or null if not detected
 */
export function detectAgentFromWorkspacesPath(path: string | undefined): string | null {
    if (isBlank(path)) {
        return null;
    }

    // Normalize path separators for cross-platform support
    const normalizedPath = path!.replace(/\\/g, "/").toLowerCase();

    // Pattern: agent-workspaces/agentX or agent-workspaces/agentX/
    const agentMatch = normalizedPath.match(/agent-workspaces\/(agent\d+|agentx)/i);
    if (agentMatch) {
        const agentId = agentMatch[1];
        // Capitalize properly: agent2 -> Agent2, agentx -> AgentX
        return agentId.charAt(0).toUpperCase() + agentId.slice(1).toLowerCase().replace("x", "X");
    }

    return null;
}

/**
 * Generate an automatic title for a block based on its metadata and type
 * @param block - The block to generate a title for
 * @param settingsEnv - Optional global settings cmd:env to check for agent identity
 */
export function generateAutoTitle(block: Block, settingsEnv?: Record<string, string>): string {
    if (!block || !block.meta) {
        return "Untitled";
    }

    const view = block.meta.view;

    switch (view) {
        case "term":
            return generateTerminalTitle(block, settingsEnv);
        case "preview":
            return generatePreviewTitle(block);
        case "codeeditor":
            return "Editor";
        case "help":
            return "Help";
        case "sysinfo":
            return "System Info";
        case "tsunami":
            return "Tsunami";
        default:
            return generateDefaultTitle(block, view);
    }
}

/**
 * Generate title for terminal blocks
 * Priority: block env vars > settings env vars > agent-workspaces > directory name
 * Note: Hostname-based detection has been removed - agent identity must be explicit via env vars
 */
function generateTerminalTitle(block: Block, settingsEnv?: Record<string, string>): string {
    const meta = block.meta!;

    // 1. Check block-level cmd:env (set via OSC 16162 from shell integration)
    // This enables per-pane agent identity
    const blockEnv = meta["cmd:env"] as Record<string, string> | undefined;
    const agentFromBlockEnv = detectAgentFromEnv(blockEnv);
    if (agentFromBlockEnv) {
        return agentFromBlockEnv;
    }

    // 2. Check global settings environment variables (fallback)
    const agentFromSettingsEnv = detectAgentFromEnv(settingsEnv);
    if (agentFromSettingsEnv) {
        return agentFromSettingsEnv;
    }

    // 3. Check for explicit agent-workspaces directory pattern
    // This is an intentional opt-in structure (e.g., /agent-workspaces/agent2/)
    const cwd = meta["cmd:cwd"] as string | undefined;
    const agentFromWorkspaces = detectAgentFromWorkspacesPath(cwd);
    if (agentFromWorkspaces) {
        return agentFromWorkspaces;
    }

    // 4. Fall back to directory basename
    if (!isBlank(cwd)) {
        return basename(cwd!) || "~";
    }

    return "Terminal";
}

/**
 * Generate title for preview blocks
 * Uses filename from meta
 */
function generatePreviewTitle(block: Block): string {
    const file = block.meta!.file;

    if (!isBlank(file)) {
        return basename(file!);
    }

    const url = block.meta!.url;
    if (!isBlank(url)) {
        try {
            const urlObj = new URL(url!);
            return urlObj.hostname || "Preview";
        } catch {
            return "Preview";
        }
    }

    return "Preview";
}


/**
 * Generate default title for unknown block types
 * Uses view name and block ID suffix
 */
function generateDefaultTitle(block: Block, view?: string): string {
    if (!isBlank(view)) {
        const viewCapitalized = view!.charAt(0).toUpperCase() + view!.slice(1);
        const blockIdShort = block.oid?.slice(0, 8) || "unknown";
        return `${viewCapitalized} (${blockIdShort})`;
    }

    const blockIdShort = block.oid?.slice(0, 8) || "unknown";
    return `Block (${blockIdShort})`;
}

/**
 * Get the basename of a path (last component)
 */
function basename(path: string): string {
    if (isBlank(path)) {
        return "";
    }

    // Handle both Unix and Windows paths
    const parts = path.split(/[/\\]/);
    const last = parts[parts.length - 1];

    return last || "";
}

/**
 * Determine if auto-title should be used for a block
 * Checks block metadata for auto-generation flag
 */
export function shouldAutoGenerateTitle(block: Block): boolean {
    if (!block || !block.meta) {
        return false;
    }

    // Check if block has explicit auto-generate setting
    const autoGenerate = block.meta["pane-title:auto"] as boolean | undefined;
    if (autoGenerate !== undefined) {
        return autoGenerate;
    }

    // Check if block has custom title - if so, don't auto-generate
    const customTitle = block.meta["pane-title"] as string | undefined;
    if (!isBlank(customTitle)) {
        return false;
    }

    // Default to auto-generate if no custom title
    return true;
}

/**
 * Get the effective title for a block
 * Returns custom title if set, otherwise auto-generates
 * @param settingsEnv - Optional global settings cmd:env to check for agent identity
 */
export function getEffectiveTitle(block: Block, autoGenerateEnabled: boolean, settingsEnv?: Record<string, string>): string {
    if (!block || !block.meta) {
        return "";
    }

    // Check for custom title first
    const customTitle = block.meta["pane-title"] as string | undefined;
    if (!isBlank(customTitle)) {
        return customTitle!;
    }

    // Auto-generate if enabled and appropriate
    if (autoGenerateEnabled && shouldAutoGenerateTitle(block)) {
        return generateAutoTitle(block, settingsEnv);
    }

    return "";
}
