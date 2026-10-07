// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Env a provider's CLI needs to find the companion installed beside it.
 * pi-acp runs whatever `pi` is on PATH unless `PI_ACP_PI_COMMAND` names one,
 * and the managed install's `.bin` is not on PATH. srv sets the same variable
 * for the session itself (`pi_beside_pi_acp` in blockcontroller/acp.rs); this
 * covers the login terminal, which the frontend opens.
 * SPEC_PI_HARNESS_VIA_PI_ACP_2026_10_06.md.
 */
export function companionCliEnv(providerId: string, cliPath: string): Record<string, string> {
    if (providerId !== "pi") return {};
    const cut = Math.max(cliPath.lastIndexOf("/"), cliPath.lastIndexOf("\\"));
    if (cut < 0) return {};
    const dir = cliPath.slice(0, cut + 1);
    const windows = /\.(cmd|exe|bat)$/i.test(cliPath);
    return { PI_ACP_PI_COMMAND: `${dir}${windows ? "pi.cmd" : "pi"}` };
}
