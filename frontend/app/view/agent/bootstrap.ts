// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ProviderDefinition } from "./providers";

/**
 * Shell types that determine which bootstrap script syntax to generate.
 *
 * The backend shell controller knows which shell it spawned (pwsh, cmd, bash, etc.)
 * and can write it to block meta as "shell:type". If unavailable, guessShellType()
 * provides a platform-based fallback.
 */
export type ShellType = "pwsh" | "powershell" | "cmd" | "bash" | "zsh" | "sh";

interface BootstrapOptions {
    version: string;
    provider: ProviderDefinition;
    shellType: ShellType;
    args: string[];
    /** Shell commands to run before CLI launch (e.g., write config files) */
    preamble?: string;
    /** Working directory to cd into before launch */
    cwd?: string;
    /** Extra CLI flags to append (from forge provider_flags) */
    extraFlags?: string;
}

/**
 * Build a one-liner bootstrap script that:
 * 1. Checks if the CLI binary exists in the version-isolated directory
 * 2. If missing, runs npm install (visible in terminal)
 * 3. Launches the CLI with env cleanup and args
 *
 * Dispatches to the correct script builder based on shell type.
 */
export function buildBootstrapScript(opts: BootstrapOptions): string {
    switch (opts.shellType) {
        case "pwsh":
        case "powershell":
            return buildPowerShellBootstrap(opts);
        case "cmd":
            return buildCmdBootstrap(opts);
        case "bash":
        case "zsh":
        case "sh":
        default:
            return buildBashBootstrap(opts);
    }
}

/**
 * Guess the shell type from the platform string.
 * Used as a fallback when the backend hasn't reported the actual shell.
 *
 * - "win32" / "windows" → "pwsh" (most common Windows shell in AgentMux)
 * - everything else → "bash"
 */
export function guessShellType(platform: string): ShellType {
    if (platform === "win32" || platform === "windows") {
        return "pwsh";
    }
    return "bash";
}

// ---------------------------------------------------------------------------
// Shell-specific script builders
// ---------------------------------------------------------------------------

/**
 * PowerShell bootstrap (works in both pwsh 7 and powershell 5.1).
 *
 * Uses $HOME, Test-Path, & invocation, and .cmd shim extension.
 */
function buildPowerShellBootstrap(opts: BootstrapOptions): string {
    const { version, provider, args, preamble, cwd, extraFlags } = opts;
    const cliDir = `$HOME/.agentmux/instances/v${version}/cli/${provider.id}`;
    const bin = `$d/node_modules/.bin/${provider.cliCommand}.cmd`;
    const pkg = `${provider.npmPackage}@${provider.pinnedVersion}`;

    let argsSuffix = args.length > 0 ? " " + args.join(" ") : "";
    if (extraFlags) {
        argsSuffix += " " + extraFlags;
    }

    let envPrefix = "";
    if (provider.unsetEnv?.length) {
        envPrefix = provider.unsetEnv.map((v) => `$env:${v}=$null`).join("; ") + "; ";
    }

    const parts: string[] = [];

    // Preamble runs as bash commands before the pwsh CLI launch.
    // Since preamble is bash-syntax (heredoc, export, mkdir -p), we skip it for pwsh.
    // The cwd is handled via Set-Location instead.
    if (cwd && !preamble) {
        parts.push(`Set-Location "${cwd}"`);
    }

    parts.push(
        `$d="${cliDir}"`,
        `$b="${bin}"`,
        `if(!(Test-Path $b)){Write-Host "Installing ${provider.displayName}..."`,
        `npm install --prefix $d ${pkg} --no-fund --no-audit}`,
        `${envPrefix}& $b${argsSuffix}`,
    );

    return parts.join("; ");
}

/**
 * cmd.exe bootstrap.
 *
 * Uses %USERPROFILE%, backslash paths, `if not exist`, @ echo suppression.
 */
function buildCmdBootstrap(opts: BootstrapOptions): string {
    const { version, provider, args, extraFlags } = opts;
    // cmd.exe uses %USERPROFILE% and backslashes
    const cliDir = `%USERPROFILE%\\.agentmux\\instances\\v${version}\\cli\\${provider.id}`;
    const bin = `%d%\\node_modules\\.bin\\${provider.cliCommand}.cmd`;
    const pkg = `${provider.npmPackage}@${provider.pinnedVersion}`;

    let argsSuffix = args.length > 0 ? " " + args.join(" ") : "";
    if (extraFlags) {
        argsSuffix += " " + extraFlags;
    }

    let envPrefix = "";
    if (provider.unsetEnv?.length) {
        envPrefix = provider.unsetEnv.map((v) => `set "${v}=" && `).join("") ;
    }

    return `@set "d=${cliDir}" && @set "b=${bin}" && @if not exist "%b%" (echo Installing ${provider.displayName}... && npm install --prefix "%d%" ${pkg} --no-fund --no-audit) && ${envPrefix}"%b%"${argsSuffix}`;
}

/**
 * Bash/zsh/sh bootstrap (Unix + WSL + Git Bash).
 *
 * Uses $HOME, [ -x ] test, forward slashes, no .cmd extension.
 */
function buildBashBootstrap(opts: BootstrapOptions): string {
    const { version, provider, args, preamble, cwd, extraFlags } = opts;
    const cliDir = `$HOME/.agentmux/instances/v${version}/cli/${provider.id}`;
    const bin = `$CLI_DIR/node_modules/.bin/${provider.cliCommand}`;
    const pkg = `${provider.npmPackage}@${provider.pinnedVersion}`;

    let argsSuffix = args.length > 0 ? " " + args.join(" ") : "";
    if (extraFlags) {
        argsSuffix += " " + extraFlags;
    }

    let envPrefix = "";
    if (provider.unsetEnv?.length) {
        envPrefix = provider.unsetEnv.map((v) => `unset ${v}`).join("; ") + "; ";
    }

    const parts: string[] = [];

    // Preamble (config file writing) runs first
    if (preamble) {
        parts.push(preamble);
    }

    parts.push(
        `CLI_DIR="${cliDir}"`,
        `CLI_BIN="${bin}"`,
        `{ [ -x "$CLI_BIN" ] || { echo "Installing ${provider.displayName}..."`,
        `npm install --prefix "$CLI_DIR" ${pkg} --no-fund --no-audit; }; }`,
    );

    // cd to cwd if specified (and no preamble already did it)
    if (cwd && !preamble) {
        parts.push(`cd ${cwd}`);
    }

    parts.push(`${envPrefix}"$CLI_BIN"${argsSuffix}`);

    return parts.join(" && ");
}
