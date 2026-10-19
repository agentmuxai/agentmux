// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The environment the lab gives the instance it starts. The lab drops every AGENTMUX_* variable of the shell it
// runs in (they would point the instance at the shell's own AgentMux), so an experiment switch of that name never
// arrives: `--env NAME=value` passes one on purpose, and the result records what the instance was given and what
// was dropped, so a condition that silently ran as the baseline shows.

/** Variables the lab sets itself: `--env` may not override them. */
export const RESERVED = ["AGENTMUX_HOME_OVERRIDE", "AGENTMUX_CDP_PORT", "FAKE_CLAUDE_SCRIPT"];

/** `NAME=value` pairs from repeated `--env`, as an object; throws on a malformed pair or a reserved name. */
export function parseEnvPairs(pairs) {
    const out = {};
    for (const pair of pairs ?? []) {
        const eq = String(pair).indexOf("=");
        const name = eq > 0 ? pair.slice(0, eq) : "";
        if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`--env needs NAME=value (got "${pair}")`);
        if (RESERVED.includes(name)) throw new Error(`--env ${name}: the lab sets that itself (${RESERVED.join(", ")})`);
        out[name] = pair.slice(eq + 1);
    }
    return out;
}

/**
 * What the instance gets: the shell's variables without AGENTMUX_*, then the lab's own, then the `--env` ones.
 * Returns the environment to spawn with and the record for result.json.
 */
export function buildInstanceEnv(shellEnv, labOwn, extra) {
    const dropped = Object.keys(shellEnv).filter((k) => k.startsWith("AGENTMUX_")).sort();
    const clean = Object.fromEntries(Object.entries(shellEnv).filter(([k]) => !k.startsWith("AGENTMUX_")));
    return {
        env: { ...clean, ...labOwn, ...extra },
        record: { passed: { ...extra }, labSet: Object.keys(labOwn).sort(), droppedFromShell: dropped },
    };
}
