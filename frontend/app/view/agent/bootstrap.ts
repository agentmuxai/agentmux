// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ProviderDefinition } from "./providers";

interface BootstrapOptions {
    version: string;
    provider: ProviderDefinition;
    isWindows: boolean;
    args: string[];
}

/**
 * Build a one-liner bootstrap script that:
 * 1. Checks if the CLI binary exists in the version-isolated directory
 * 2. If missing, runs npm install (visible in terminal)
 * 3. Launches the CLI with env cleanup and args
 *
 * The script is injected into the terminal via ControllerInputCommand,
 * so the user sees install progress, errors, and output directly.
 */
export function buildBootstrapScript(opts: BootstrapOptions): string {
    const { version, provider, isWindows, args } = opts;
    const cliDir = `$HOME/.agentmux/instances/v${version}/cli/${provider.id}`;
    const pkg = `${provider.npmPackage}@${provider.pinnedVersion}`;

    // Build env cleanup prefix
    let envPrefix = "";
    if (provider.unsetEnv?.length) {
        if (isWindows) {
            envPrefix = provider.unsetEnv.map((v) => `$env:${v}=$null`).join("; ") + "; ";
        } else {
            envPrefix = provider.unsetEnv.map((v) => `unset ${v}`).join("; ") + "; ";
        }
    }

    // Build args suffix
    const argsSuffix = args.length > 0 ? " " + args.join(" ") : "";

    if (isWindows) {
        // PowerShell one-liner
        const bin = `$d/node_modules/.bin/${provider.cliCommand}.cmd`;
        return [
            `$d="${cliDir}"`,
            `$b="${bin}"`,
            `if(!(Test-Path $b)){Write-Host "Installing ${provider.displayName}..."`,
            `npm install --prefix $d ${pkg} --no-fund --no-audit}`,
            `${envPrefix}& $b${argsSuffix}`,
        ].join("; ");
    } else {
        // Bash one-liner
        const bin = `$CLI_DIR/node_modules/.bin/${provider.cliCommand}`;
        return [
            `CLI_DIR="${cliDir}"`,
            `CLI_BIN="${bin}"`,
            `{ [ -x "$CLI_BIN" ] || { echo "Installing ${provider.displayName}..."`,
            `npm install --prefix "$CLI_DIR" ${pkg} --no-fund --no-audit; }; }`,
            `${envPrefix}"$CLI_BIN"${argsSuffix}`,
        ].join(" && ");
    }
}
