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

    // hsl()/hsla() — needed since this PR's own callers (hueToHeaderBg,
    // NON_AGENT_DEFAULT_HEADER_BG) pass hsl() strings into
    // pickReadableTextColor, which relies on this parser. Without this
    // branch those calls silently returned null and computed no text color
    // at all (reagent P1, PR #3452).
    const hslMatch = c.match(
        /^hsla?\(\s*([\d.]+)(?:deg)?\s*[, ]\s*([\d.]+)%\s*[, ]\s*([\d.]+)%\s*(?:[,/]\s*([\d.]+%?)\s*)?\)$/
    );
    if (hslMatch) {
        const h = Number(hslMatch[1]);
        const s = Number(hslMatch[2]) / 100;
        const l = Number(hslMatch[3]) / 100;
        let a = 1;
        if (hslMatch[4] != null) {
            a = hslMatch[4].endsWith("%") ? Number(hslMatch[4].slice(0, -1)) / 100 : Number(hslMatch[4]);
        }
        if ([h, s, l, a].some((n) => Number.isNaN(n))) {
            return null;
        }
        return { ...hslToRgb(h, s, l), a };
    }

    return null;
}

/** HSL (h in degrees, s/l in 0-1) to RGB (0-255 each). Standard conversion. */
function hslToRgb(h: number, s: number, l: number): { r: number; g: number; b: number } {
    const hue = ((h % 360) + 360) % 360;
    const c = (1 - Math.abs(2 * l - 1)) * s;
    const x = c * (1 - Math.abs(((hue / 60) % 2) - 1));
    const m = l - c / 2;
    let rp = 0;
    let gp = 0;
    let bp = 0;
    if (hue < 60) {
        [rp, gp, bp] = [c, x, 0];
    } else if (hue < 120) {
        [rp, gp, bp] = [x, c, 0];
    } else if (hue < 180) {
        [rp, gp, bp] = [0, c, x];
    } else if (hue < 240) {
        [rp, gp, bp] = [0, x, c];
    } else if (hue < 300) {
        [rp, gp, bp] = [x, 0, c];
    } else {
        [rp, gp, bp] = [c, 0, x];
    }
    return {
        r: Math.round((rp + m) * 255),
        g: Math.round((gp + m) * 255),
        b: Math.round((bp + m) * 255),
    };
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
 * Pick readable header text color (`#000000`/`#ffffff`) for a given
 * background. Replaces the old per-agent `DEFAULT_AGENT_TEXT_COLORS`
 * hardcoded table (decommissioned — see
 * `docs/specs/SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md`): computed
 * generically from whatever background color is actually in use (the
 * agent's persisted `frame:activebordercolor`, or an explicit "Pane Color"
 * hue) instead of needing a hardcoded entry per known agent name.
 * Unparseable/blank input returns null — caller keeps the theme default.
 *
 * Picks whichever of black/white gives the HIGHER WCAG contrast ratio,
 * computed directly — not a `luminance > 0.5` cutoff. That cutoff is wrong:
 * the real black/white crossover is where the two candidates' contrast
 * ratios are equal, which works out to background luminance ≈0.179, not
 * 0.5. A fixed 0.5 threshold picks white for the whole 0.179–0.5 range even
 * though black reads better there — e.g. `#f59e0b` (amber, one of this
 * app's own AGENT_COLOR_PALETTE entries), L≈0.44: white-on-amber is
 * ~2.15:1 (fails WCAG AA), black-on-amber is ~9.78:1. codex P1, PR #3452.
 */
export function pickReadableTextColor(bgColor: string | null | undefined): string | null {
    const rgba = parseCssColor(bgColor);
    if (!rgba) {
        return null;
    }
    const bgLum = relativeLuminance(rgba);
    // WCAG contrast ratio: (lighter + 0.05) / (darker + 0.05).
    const contrastWithWhite = 1.05 / (bgLum + 0.05);
    const contrastWithBlack = (bgLum + 0.05) / 0.05;
    return contrastWithBlack >= contrastWithWhite ? "#000000" : "#ffffff";
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
