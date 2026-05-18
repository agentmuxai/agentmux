// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMux error catalog — mirrors `agentmux-common::AgentMuxError`.
 *
 * The Rust enum is the source of truth for codes; this file maps each
 * stable code to a user-facing title, message renderer, and optional
 * recovery hint. Every entry MUST exist in both places.
 *
 * See `docs/specs/SPEC_ERROR_CATALOG_2026_05_17.md` for the full design.
 */

export interface ErrorEntry {
    /** Short headline rendered in the banner's bold first line. */
    title: string;
    /** Sentence-case description; receives the wire `details` object. */
    message: (details: Record<string, unknown>) => string;
    /** Italicized recovery hint under the message. Static string or a
     *  function of `details` for entries that need to surface
     *  payload-specific text (e.g. a system install command). */
    retry?: string | ((details: Record<string, unknown>) => string | undefined);
}

const noPath = (v: unknown): string => (typeof v === "string" && v ? v : "the affected location");
const str = (v: unknown, fallback = ""): string => (typeof v === "string" ? v : fallback);
const num = (v: unknown, fallback = 0): number => (typeof v === "number" ? v : fallback);

export const ERROR_CATALOG: Record<string, ErrorEntry> = {
    // ── Filesystem / I/O ────────────────────────────────────────────
    "AMX-IO-001": {
        title: "Device out of space",
        message: (d) => `Couldn't write to ${noPath(d.path)} — no space left on the disk.`,
        retry: "Free up some space and try again.",
    },
    "AMX-IO-002": {
        title: "Permission denied",
        message: (d) => `AgentMux can't access ${noPath(d.path)}.`,
        retry: "Check the folder's permissions, or relaunch as the user that created it.",
    },
    "AMX-IO-003": {
        title: "Path not found",
        message: (d) => `Couldn't find ${noPath(d.path)}.`,
    },
    "AMX-IO-004": {
        title: "Path blocked",
        message: (d) => `Refused to access ${noPath(d.path)} — looks like a path-traversal attempt.`,
    },

    // ── Persistence ─────────────────────────────────────────────────
    "AMX-STORE-001": {
        title: "Database migration failed",
        message: (d) => `Schema migration ${num(d.from)}→${num(d.to)} failed: ${str(d.message, "unknown error")}.`,
        retry: "Report this — your data is intact but the new version can't read it yet.",
    },
    "AMX-STORE-002": {
        title: "Concurrent edit detected",
        message: (d) => `Someone else updated ${str(d.oid, "this item")} between your fetch and save.`,
        retry: "Reload and try again.",
    },

    // ── Provider CLI ────────────────────────────────────────────────
    "AMX-CLI-001": {
        title: "CLI not installed",
        message: (d) => `${str(d.provider, "This agent")} isn't installed yet.`,
        retry: "Click Install now in the agent picker.",
    },
    "AMX-CLI-002": {
        title: "Install failed",
        message: (d) => `npm install of ${str(d.package, "the package")} failed: ${str(d.message, "unknown error")}.`,
        retry: "Check your internet connection and try again.",
    },
    "AMX-CLI-003": {
        title: "Installation incomplete",
        message: (d) =>
            `${str(d.provider, "The CLI")} was installed but the expected binary is missing at ${str(d.expected_path, "the install path")}.`,
        retry: "Reinstall the agent — the package may be misconfigured.",
    },
    "AMX-CLI-004": {
        title: "CLI not on PATH",
        message: (d) =>
            `${str(d.cli, "The CLI")} isn't on your PATH and ${str(d.provider, "this agent")} can't be auto-installed.`,
        // Surface the platform-specific install command the provider
        // ships in its catalog entry (e.g. `pip install kimi-cli`)
        // so the user has a copy-pasteable next step.
        retry: (d) => {
            const hint = str(d.install_hint);
            return hint ? `Install it manually: ${hint}` : "Install the CLI manually and add it to your PATH.";
        },
    },

    // ── Auth ────────────────────────────────────────────────────────
    "AMX-AUTH-001": {
        title: "Interactive login required",
        message: (d) => `${str(d.provider, "This provider")} needs a real terminal for its OAuth login.`,
        retry: "Use the Connect button in the launch modal — that wires up a PTY.",
    },
    "AMX-AUTH-002": {
        title: "Login timed out",
        message: (d) => `${str(d.provider, "Login")} didn't complete within ${num(d.seconds, 300)}s.`,
        retry: "Try again. Keep the browser window open until the success page appears.",
    },

    // ── Network ─────────────────────────────────────────────────────
    "AMX-NET-001": {
        title: "Network request failed",
        message: (d) => {
            const status = typeof d.status === "number" ? ` (HTTP ${d.status})` : "";
            return `Request to ${str(d.url, "the server")}${status} failed: ${str(d.message, "unknown error")}.`;
        },
        retry: "Check your connection and retry.",
    },

    // ── Lifecycle ───────────────────────────────────────────────────
    "AMX-LIFECYCLE-001": {
        title: "Backend failed to start",
        message: (d) => `The sidecar couldn't bind port ${num(d.port)}: ${str(d.message, "unknown error")}.`,
        retry: "Close other AgentMux instances on that port and relaunch.",
    },
    "AMX-LIFECYCLE-002": {
        title: "AgentMux is already running",
        message: (d) => `Another AgentMux instance (pid ${num(d.pid)}) holds the single-instance lock.`,
        retry: "Close the existing instance or relaunch its window from the tray.",
    },

    // ── Fallback ────────────────────────────────────────────────────
    "AMX-LEGACY": {
        title: "Something went wrong",
        message: (d) => str(d.message, "An unexpected error occurred."),
    },
};
