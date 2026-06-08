// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool-tones parameter mapping — every tool call resolves to a short
 * "syllable" of 1–3 tones drawn from a G major pentatonic scale.
 *
 * Spec: docs/specs/SPEC_AGENT_TOOL_CALL_TONES_2026_06_05.md §3.
 *
 * Design properties:
 *  - **Deterministic.** Same tool name → same syllable, every time.
 *    Users learn the alphabet by exposure.
 *  - **Bounded.** All output (curated + hashed) lives in the same
 *    pentatonic note set — no dissonance even when 6 tools overlap.
 *  - **Lightweight.** Pure functions, no I/O, no allocation per call
 *    beyond the returned record.
 *  - **No state.** This module knows nothing about the AudioContext;
 *    `tool-tones-player.ts` consumes the params and synthesizes.
 */

export interface SyllableParams {
    /** 1–3 frequencies in Hz, played in order with `gapMs` between them. */
    tones: number[];
    /** Per-tone duration in ms (40–80 is the spec'd window). */
    durationMs: number;
    /** Silent gap between tones in ms (10–24 is the spec'd window). */
    gapMs: number;
    /** Oscillator wave shape. Sine for warm tones; triangle for "machine" tones. */
    wave: OscillatorType;
}

// ─── Pentatonic scale ──────────────────────────────────────────────────
// G major pentatonic across two octaves: G3, A3, B3, D4, E4, G4, A4, B4,
// D5, E5, G5. All curated syllables draw from this set; the hash
// fallback also lives inside it.

const G3 = 196;
const A3 = 220;
const B3 = 247;
const D4 = 294;
const E4 = 330;
const G4 = 392;
const A4 = 440;
const B4 = 494;
const D5 = 587;
const E5 = 659;
const G5 = 784;

/**
 * Semitone offsets from G3 for the pentatonic notes we use in the
 * hash fallback. Eight entries chosen so a 3-bit slice of the hash
 * picks one cleanly.
 */
const PENTATONIC_DEGREES = [0, 2, 4, 7, 9, 12, 14, 16];

/**
 * Convert a pentatonic degree index (0–7) to a frequency in the G
 * pentatonic scale, anchored at G3 (196 Hz).
 */
function degreeToHz(degreeIndex: number): number {
    const semitones = PENTATONIC_DEGREES[degreeIndex % PENTATONIC_DEGREES.length];
    return G3 * Math.pow(2, semitones / 12);
}

// ─── Curated palette ──────────────────────────────────────────────────
// Each canonical tool gets a hand-tuned syllable that reflects its
// semantic role. See spec §3.2.1 and Appendix A for the table.

const CURATED: Record<string, SyllableParams> = {
    // Soft, descending major-2nd — "looking."
    Read: { tones: [B4, A4], durationMs: 60, gapMs: 14, wave: "sine" },
    // Twin same-pitch ticks — "scanning."
    Grep: { tones: [E5, E5], durationMs: 45, gapMs: 18, wave: "sine" },
    // Rising minor-2nd — "results coming."
    Glob: { tones: [D5, E5], durationMs: 45, gapMs: 18, wave: "sine" },
    // Three-tone up-down-up — "the fix gesture."
    Edit: { tones: [A4, B4, A4], durationMs: 50, gapMs: 12, wave: "sine" },
    // Rising perfect-5th — "creation."
    Write: { tones: [G4, D5], durationMs: 60, gapMs: 18, wave: "sine" },
    // Same 5th an octave down + triangle wave — "the machine."
    Bash: { tones: [G3, D4], durationMs: 70, gapMs: 18, wave: "triangle" },
    // Rising 4th, high — "delegation upward."
    Task: { tones: [D5, G5], durationMs: 55, gapMs: 14, wave: "sine" },
    // Out-and-back 5th, three tones — "sub-agent dispatch."
    Agent: { tones: [G4, D5, G4], durationMs: 55, gapMs: 14, wave: "sine" },
};

/**
 * FNV-1a-like hash for deterministic, allocation-light tool-name
 * fingerprinting. We only need ~12 bits of entropy for the syllable
 * picks, so a single 32-bit pass is plenty.
 */
function hashTool(tool: string): number {
    let h = 2166136261 >>> 0;
    for (let i = 0; i < tool.length; i++) {
        h = (h ^ tool.charCodeAt(i)) >>> 0;
        h = Math.imul(h, 16777619) >>> 0;
    }
    return h;
}

/**
 * Hash fallback for unknown tools. Returns a syllable drawn from the
 * pentatonic set so it never sours against curated tones, even when
 * overlapping.
 */
export function hashToolToParams(tool: string): SyllableParams {
    const h = hashTool(tool);
    const a = h & 0x7; // 3 bits for first tone
    const b = (h >>> 3) & 0x7; // 3 bits for second tone
    const wave: OscillatorType = ((h >>> 6) & 1) === 0 ? "sine" : "triangle";
    // 45–76 ms duration window
    const durationMs = 45 + ((h >>> 7) & 0x1f);
    // 12–19 ms gap window
    const gapMs = 12 + ((h >>> 12) & 0x7);
    return {
        tones: [degreeToHz(a), degreeToHz(b)],
        durationMs,
        gapMs,
        wave,
    };
}

/**
 * Resolve a tool name to its syllable. Curated tools hit the
 * hand-tuned palette; everything else (MCP tools, custom providers,
 * misspellings) is hashed.
 */
export function paramsForTool(tool: string): SyllableParams {
    return CURATED[tool] ?? hashToolToParams(tool);
}

/** Test/debug helper — the immutable curated palette. */
export function __getCurated(): Readonly<Record<string, SyllableParams>> {
    return CURATED;
}
