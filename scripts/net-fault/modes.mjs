// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The fault proxy's mode logic, with no sockets: parsing modes and durations,
 * matching hosts against rules, and resolving a `flap` to what it is doing now.
 *
 * Part of the fault injection in docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §7.1.
 */

/** What a connection can be given at one moment. */
export const BASE_MODES = ["pass", "refuse", "blackhole", "dns"];

const UNITS = { ms: 1, s: 1000, m: 60_000, h: 3_600_000 };

/** `500ms`, `30s`, `2m`, `1h`, or a bare number of seconds, in milliseconds. */
export function parseDuration(text) {
    const m = /^(\d+(?:\.\d+)?)(ms|s|m|h)?$/.exec(String(text).trim());
    if (!m) throw new Error(`not a duration: "${text}" (use e.g. 500ms, 30s, 2m)`);
    const ms = Math.round(Number(m[1]) * UNITS[m[2] ?? "s"]);
    if (ms <= 0) throw new Error(`duration must be positive: "${text}"`);
    return ms;
}

/**
 * `pass`, `refuse`, `blackhole`, `dns`, or `flap:<up>/<down>[:<downMode>]`.
 * A flap starts up (`pass`) for `<up>`, then is `<downMode>` (default `refuse`)
 * for `<down>`, and repeats.
 */
export function parseMode(text) {
    const s = String(text).trim().toLowerCase();
    if (BASE_MODES.includes(s)) return { kind: s };
    const m = /^flap:([^/]+)\/([^:]+)(?::(\w+))?$/.exec(s);
    if (!m) throw new Error(`unknown mode "${text}" (use ${BASE_MODES.join(", ")} or flap:<up>/<down>[:<downMode>])`);
    const down = m[3] ?? "refuse";
    if (!BASE_MODES.includes(down) || down === "pass")
        throw new Error(`a flap's down mode must be refuse, blackhole or dns, not "${down}"`);
    return { kind: "flap", upMs: parseDuration(m[1]), downMs: parseDuration(m[2]), down };
}

export function formatMode(mode) {
    if (mode.kind !== "flap") return mode.kind;
    const d = (ms) => (ms % 1000 === 0 ? `${ms / 1000}s` : `${ms}ms`);
    return `flap:${d(mode.upMs)}/${d(mode.downMs)}${mode.down === "refuse" ? "" : `:${mode.down}`}`;
}

/** A host glob: `*` is any run of characters (dots too), `?` one; case-insensitive. */
export function globToRegExp(glob) {
    const body = String(glob)
        .toLowerCase()
        .replace(/[.+^${}()|[\]\\]/g, "\\$&")
        .replace(/\*/g, ".*")
        .replace(/\?/g, ".");
    return new RegExp(`^${body}$`);
}

export function hostMatches(glob, host) {
    return globToRegExp(glob).test(String(host).toLowerCase());
}

/** The proxy's whole configuration: a default mode, and per-host rules tried in order. */
export function createState(initial = "pass", now = Date.now()) {
    return { default: { mode: parseMode(initial), since: now }, rules: [] };
}

/** Sets the default mode, or the rule for `host` (replacing one for the same glob in place). */
export function setMode(state, modeText, host, now = Date.now()) {
    const entry = { mode: parseMode(modeText), since: now };
    if (!host) {
        state.default = entry;
        return state;
    }
    const existing = state.rules.find((r) => r.host === host.toLowerCase());
    if (existing) Object.assign(existing, entry);
    else state.rules.push({ host: host.toLowerCase(), ...entry });
    return state;
}

export function clearRules(state) {
    state.rules = [];
    return state;
}

/** The rule (or the default) that governs `host`. */
export function entryFor(state, host) {
    return state.rules.find((r) => hostMatches(r.host, host)) ?? state.default;
}

/** What an entry is doing at `now`: a base mode, with a flap resolved to its current phase. */
export function effective(entry, now = Date.now()) {
    const { mode } = entry;
    if (mode.kind !== "flap") return mode.kind;
    const phase = (now - entry.since) % (mode.upMs + mode.downMs);
    return phase < mode.upMs ? "pass" : mode.down;
}

export function decide(state, host, now = Date.now()) {
    return effective(entryFor(state, host), now);
}

/** Milliseconds until any flap changes phase, or null when nothing will change by itself. */
export function nextChangeIn(state, now = Date.now()) {
    let next = null;
    for (const entry of [state.default, ...state.rules]) {
        const { mode } = entry;
        if (mode.kind !== "flap") continue;
        const phase = (now - entry.since) % (mode.upMs + mode.downMs);
        const left = phase < mode.upMs ? mode.upMs - phase : mode.upMs + mode.downMs - phase;
        next = next === null ? left : Math.min(next, left);
    }
    return next;
}

/**
 * True when every host is refused right now. The proxy then stops listening, so
 * clients get a real `ECONNREFUSED` from their connect, not a reset after it.
 */
export function refusesEverything(state, now = Date.now()) {
    return [state.default, ...state.rules].every((e) => effective(e, now) === "refuse");
}

/** A JSON-friendly snapshot for `/status` and the log. */
export function describe(state, now = Date.now()) {
    const one = (e) => ({ mode: formatMode(e.mode), now: effective(e, now) });
    return { default: one(state.default), rules: state.rules.map((r) => ({ host: r.host, ...one(r) })) };
}
