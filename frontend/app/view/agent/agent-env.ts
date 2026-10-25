// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The extra environment variables a definition launches with: the `env` content
// blob, one `KEY=VALUE` per line, `#` for a comment. The launch (agent-model.ts)
// and the Environment editor read it with this one parser, so the editor shows
// exactly the lines the launch would skip. srv's rules for what an agent may
// write are in crates/srv/src/backend/agent_env.rs; the operator is not bound
// by them.

/** The `agent_content` type that holds a definition's environment. */
export const ENV_CONTENT_TYPE = "env";

const ENV_NAME = /^[a-zA-Z_][a-zA-Z0-9_]*$/;
const RESERVED_PREFIX = "AGENTMUX_";

export interface IgnoredEnvLine {
    /** 1-based, as the editor counts them. */
    number: number;
    text: string;
    reason: string;
}

export interface ParsedEnv {
    /** What the launch gets; a name given twice has its last value. */
    vars: Record<string, string>;
    /** Lines the launch skips. */
    ignored: IgnoredEnvLine[];
    /** Names AgentMux reserves, which it may override whatever is given here. */
    reserved: string[];
}

/** Read `text` the way the launch does: a line is trimmed, blank and `#` lines skipped, the key is everything before the first `=`. */
export function parseEnvText(text: string | undefined | null): ParsedEnv {
    const parsed: ParsedEnv = { vars: {}, ignored: [], reserved: [] };
    (text ?? "").split("\n").forEach((raw, index) => {
        const line = raw.trim();
        if (!line || line.startsWith("#")) return;
        const eq = line.indexOf("=");
        const skip = (reason: string) => parsed.ignored.push({ number: index + 1, text: line, reason });
        if (eq < 1) return skip("no NAME= before the value");
        const key = line.substring(0, eq);
        if (!ENV_NAME.test(key)) return skip(`${key} is not a name: use letters, digits and underscores, not starting with a digit`);
        parsed.vars[key] = line.substring(eq + 1);
        if (key.toUpperCase().startsWith(RESERVED_PREFIX) && !parsed.reserved.includes(key)) parsed.reserved.push(key);
    });
    return parsed;
}
