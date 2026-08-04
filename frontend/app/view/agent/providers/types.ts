// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A system tool the provider's CLI needs at runtime — beyond Node
 * itself, which is checked separately by `check_nodejs_available`.
 * Examples: Claude Code calls `git` from inside session-start; the
 * GitHub Copilot CLI wraps `gh`.
 *
 * Probed pre-launch via the `resolve_prereqs` RPC. Missing prereqs
 * open the `AgentPrereqModal` with platform-aware install links.
 * See docs/specs/SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md.
 */
export interface SystemPrereq {
    /** Binary name to look up via `where` (Windows) / `which` (Unix). */
    tool: string;
    /** Display label in the banner. Defaults to `tool` if omitted. */
    label?: string;
    /** Per-platform official install URLs. Curated landing pages so
     *  the link reads as intentional, not a generic Google search. */
    installUrls: {
        windows: string;
        macos: string;
        linux: string;
    };
    /** Anchor text shown for the install link, per platform. Falls
     *  back to "Install {label}" when omitted. */
    installLinkText?: {
        windows?: string;
        macos?: string;
        linux?: string;
    };
}

/** One selectable model for a provider's `--model` flag. */
export interface ProviderModel {
    value: string;          // the model string passed to the CLI (e.g. "opus", "gpt-5.5")
    label: string;          // UI label
    default?: boolean;      // the default model for this provider
    description?: string;   // optional one-liner shown in the /model picker
    aliases?: string[];     // optional /model aliases (e.g. "claude-opus")
}

export interface ProviderDefinition {
    id: string;
    displayName: string;
    cliCommand: string;
    defaultArgs: string[];
    styledArgs: string[];        // CLI flags for JSON streaming mode (documentation; use launchArgs for actual invocation)
    outputFormat: "claude-stream-json" | "gemini-json" | "codex-json" | "kimi-stream-json" | "acp" | "raw";
    styledOutputFormat: "claude-stream-json" | "gemini-json" | "codex-json" | "kimi-stream-json" | "acp";
    authType: "oauth" | "api-key";
    authCheckCommand: string[];  // e.g. ["auth", "status", "--json"]
    authLoginCommand: string[];  // e.g. ["auth", "login"]
    npmPackage: string;          // npm package name for local install
    pinnedVersion: string;       // version to install ("latest" or specific)
    docsUrl: string;
    windowsInstallCommand: string;  // official installer for Windows (powershell)
    unixInstallCommand: string;      // official installer for macOS/Linux (bash)
    icon: string;
    unsetEnv?: string[];         // env vars to unset before launching (e.g. nested-session guards)
    // Auth isolation — each provider gets its own versioned config dir
    authConfigDirEnvVar: string;        // env var that redirects the provider's config/auth dir
    authDirName: string;                // subdir name under {dataDir}/auth/ (e.g. "claude")
    authExtraEnv?: Record<string, string>;  // extra env vars needed for auth isolation (e.g. GEMINI_FORCE_FILE_STORAGE)
    // Launch args — the complete CLI args for a single turn (replaces hardcoded ["-p", ...styledArgs])
    // The user message is written to subprocess stdin; these args put the CLI in non-interactive mode.
    launchArgs: string[];
    // Resume flag — how to pass a session ID for multi-turn continuity.
    // null means this provider does not support simple-flag resume (e.g. Codex uses a subcommand).
    resumeFlag: string | null;
    // JSON field name containing the session/thread ID in the CLI's init event.
    sessionIdField: string;
    // Controller type: "persistent" keeps a long-running process with stdin streaming,
    // "subprocess" spawns a fresh process per turn with --resume,
    // "acp" uses the Agent Client Protocol (JSON-RPC 2.0 over stdio).
    controllerType: "persistent" | "subprocess" | "acp";
    // Launch args for persistent mode (--input-format stream-json, no -p).
    // Only used when controllerType is "persistent".
    persistentLaunchArgs?: string[];
    // Spawn the auth login subprocess under a PTY (instead of plain
    // piped stdio). Required by providers whose auth subcommand checks
    // `isatty()` and refuses to run otherwise — currently OpenClaw's
    // `openclaw models auth login --provider <id>`. The host's
    // `run_cli_login` reads this flag and chooses the PTY branch.
    requiresLoginTty?: boolean;
    // Set when this provider's login subcommand is known to NEVER print a
    // scrapeable OAuth URL (distinct from requiresLoginTty — OpenClaw needs a
    // TTY but does print a URL through it). When true, runProviderLogin's
    // tier-1 PTY/pipe URL-capture attempt is skipped entirely instead of
    // burning its ~15s capture timeout on a documented dead end.
    //
    // No catalog entry sets this anymore: Claude was the only one, and its
    // pinned CLI (2.1.198+) now prints the authorize URL under our PTY spawn
    // (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §2, 2026-08-03 probes) —
    // so the flag was dropped there. Kept as a behavior-gate mechanism: a
    // future provider whose CLI genuinely prints nothing should set it rather
    // than reintroducing per-call-site skipTier1 hardcoding.
    headlessLoginUrlUnsupported?: boolean;
    /** System tools the provider's CLI needs at runtime. Probed before
     *  the user launches the agent so we can show install links
     *  instead of letting the CLI fail with cryptic stderr. */
    systemPrereqs?: SystemPrereq[];
    /**
     * The model's maximum input token capacity (context window size).
     * Used by the composer strip to render a context-fill progress bar.
     * Omit for providers whose context window is unknown or variable.
     */
    contextWindow?: number;
    /**
     * AgentMux-side model choices for this provider's `--model` flag. Drives the
     * `/model` slash command (and, for Claude, the control-bar dropdown). Mark one
     * `default`. Omit for providers whose model is chosen in their own config
     * (muxcode/openclaw/pi/copilot) — those show no AgentMux model picker.
     * NOTE: model strings move with the upstream provider — keep current; see
     * docs/providers/PROVIDER_MODELS_EFFORT_SETTINGS_2026-06.md.
     */
    models?: ProviderModel[];
}
