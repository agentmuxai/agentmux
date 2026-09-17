#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// pane-load.mjs — on-demand PTY output load, for reproducing cross-pane input
// lag BY HAND.
//
// Run it in one pane, type in another while it floods, and see whether your
// keystrokes stutter. That is the whole job: this script does not measure
// anything about the other pane — a human's fingers are the instrument — it
// just produces load that is heavy and varied enough to actually provoke the
// symptom, on demand, for a window long enough to type in.
//
// WHY NOT `yes`: `bench-term-cross-pane.mjs` floods pane A with `yes`, which
// is a stream of "y\n" — maximal bytes, near-zero terminal work. xterm.js
// parses that almost for free. Real output that makes a UI feel bad is
// ESCAPE-SEQUENCE heavy: cursor jumps, colour changes, in-place redraws, the
// things a build log or a TUI emits. Those cost parser and renderer time per
// byte, not just bandwidth. A repro built on `yes` can be perfectly green
// while the thing users complain about is untouched — see
// docs/analysis/ANALYSIS_CROSS_PANE_INPUT_DELAY_UNDER_OUTPUT_LOAD_2026_09_04.md
// §3.5 and its 09-15 follow-up.
//
// THE MODE THAT REPRODUCES IS A CLUE, not a detail. Each one leans on a
// different mechanism, so "it only stutters under --mode spinner" and "it
// stutters under --mode text" point at different subsystems:
//
//   text     bulk bytes, few writes   → WS egress, FileStore write-through,
//                                       xterm's raw parse cost
//   paint    bulk escape sequences    → xterm parser + renderer; forces real
//                                       repaint work per frame
//   spinner  many tiny writes         → per-write/per-frame overhead: WS
//                                       frames, event dispatch, the coalescing
//                                       path (PTY_COALESCE_WINDOW) — ONLY at
//                                       --unpaced or a --throttle-ms tuned to
//                                       exceed it; see the note below
//   mixed    rotates all three        → default; use it to find out IF it
//                                       reproduces, then narrow with one mode
//
// SPINNER'S CLAIM NEEDS A CAVEAT (Codex P2 on PR #3286). The backend's own
// output flusher (`run_pty_output_flusher`, `shell/lifecycle.rs`) coalesces
// PTY reads for up to `PTY_COALESCE_WINDOW` (20ms) or `PTY_COALESCE_MAX_BYTES`
// (256 KiB), whichever comes first, before broadcasting even one WS frame.
// At this script's DEFAULT auto-paced rate (~6.4 MB/s), every mode stays well
// under that byte threshold, so the 20ms window is what triggers each flush —
// meaning text, paint, and spinner all produce roughly the SAME downstream
// broadcast rate regardless of how many tiny writes spinner made to get
// there. The "many small writes" story is real on the PRODUCER side (this
// process's own writes to the PTY) but does not by itself prove anything
// about WS-frame count downstream unless the write rate is fast enough to
// blow through the coalescing window — i.e. `--unpaced`, or a `--throttle-ms`
// picked deliberately low enough to matter, not the default.
//
// Usage (inside a terminal pane, or an agent pane's shell drawer):
//
//   node tools/tests/pane-load.mjs                      # mixed, 10s
//   node tools/tests/pane-load.mjs --mode paint --secs 15
//   node tools/tests/pane-load.mjs --mode spinner --throttle-ms 1
//
// For the AGENT-PANE rendering path (markdown/SolidJS, NOT xterm) this script
// is the wrong tool — see tools/tests/README.md for that scenario.

const args = process.argv.slice(2);

function getArg(name, fallback) {
    const i = args.indexOf(name);
    return i !== -1 && args[i + 1] !== undefined ? args[i + 1] : fallback;
}

if (args.includes("--help") || args.includes("-h")) {
    process.stdout.write(`
node tools/tests/pane-load.mjs [options]

  --mode <m>          text | paint | spinner | mixed   (default: mixed)
  --secs <n>          how long to flood, in seconds    (default: 10)
  --countdown <n>     seconds before the flood starts  (default: 3)
                      — time to move focus to the other pane
  --cols <n>          assumed terminal width           (default: $COLUMNS or 100)
  --rows <n>          rows per repaint frame, paint mode (default: 24)
  --throttle-ms <n>   fixed sleep between writes       (default: auto-paced)
                      Overrides pacing. Use it to isolate write COUNT from
                      byte volume.
  --unpaced           flood as fast as the PTY accepts. Hits --max-mb in a
                      fraction of a second, so the typing window all but
                      disappears — for pathological/backpressure testing, not
                      for the hand repro.
  --max-mb <n>        stop early after this many MB    (default: 64)
                      SAFETY VALVE, not a tuning knob — a pane's terminal
                      output is written through to the FileStore for
                      scrollback persistence, so an unbounded flood is an
                      unbounded write to the user's disk. Uncapped, the modes
                      here sustain 250-400 MB/s into a pipe. Raise it
                      deliberately, knowing where the bytes land.
  --label <s>         marker printed at start/end, so a run can be found in
                      \`muxlog srv grep\` alongside backend timings
  --help              this message

Reproducing cross-pane input lag:
  1. Open two panes side by side.
  2. Run this in pane A.
  3. While the countdown runs, put your cursor in pane B.
  4. Type continuously in B for the whole flood. Note any stutter, dropped
     characters, or lag between keypress and echo.
  5. Re-run with a single --mode to find which load class provokes it.
`);
    process.exit(0);
}

const MODE = getArg("--mode", "mixed");
const SECS = Number(getArg("--secs", "10"));
const COUNTDOWN = Number(getArg("--countdown", "3"));
const COLS = Number(getArg("--cols", process.env.COLUMNS || "100"));
const ROWS = Number(getArg("--rows", "24"));
const THROTTLE_MS = Number(getArg("--throttle-ms", "0"));
const MAX_MB = Number(getArg("--max-mb", "64"));
const UNPACED = args.includes("--unpaced");
const THROTTLE_GIVEN = args.includes("--throttle-ms");
const LABEL = getArg("--label", "");

const VALID_MODES = ["text", "paint", "spinner", "mixed"];
if (!VALID_MODES.includes(MODE)) {
    process.stderr.write(`pane-load: unknown --mode ${MODE} (expected: ${VALID_MODES.join(" | ")})\n`);
    process.exit(2);
}
if (!Number.isFinite(SECS) || SECS <= 0) {
    process.stderr.write(`pane-load: --secs must be a positive number\n`);
    process.exit(2);
}
// ReAgent P1 on PR #3286: an unvalidated --max-mb defeats the disk-safety
// valve silently rather than loudly. A non-numeric value makes maxBytes NaN,
// and `bytes >= NaN` is always false in JS — so the cap this flag exists to
// enforce would simply never fire, for exactly the failure mode (bad/typo'd
// argument) a safety valve is supposed to catch.
if (!Number.isFinite(MAX_MB) || MAX_MB <= 0) {
    process.stderr.write(`pane-load: --max-mb must be a positive number\n`);
    process.exit(2);
}

const ESC = "\x1b[";
const COLORS = [31, 32, 33, 34, 35, 36, 91, 92, 93, 94, 95, 96];

/** Backpressure-aware write: resolves once the PTY has actually taken it.
 *  Without honouring `drain`, a fast loop just buries bytes in Node's own
 *  buffer and reports a throughput the terminal never saw. */
function write(s) {
    if (process.stdout.write(s)) return null;
    return new Promise((resolve) => process.stdout.once("drain", resolve));
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Bulk bytes, few writes. Long printable lines, no escapes. */
function textFrame(n) {
    const body = [];
    for (let i = 0; i < 16; i++) {
        const filler = String.fromCharCode(97 + ((n + i) % 26)).repeat(Math.max(1, COLS - 24));
        body.push(`[${String(n).padStart(8, "0")}:${String(i).padStart(2, "0")}] ${filler}`);
    }
    return body.join("\n") + "\n";
}

/** Escape-sequence heavy: absolute cursor moves, colour churn, in-place
 *  redraw of a whole screenful. This is the one that costs xterm real parser
 *  and renderer time rather than just bandwidth. */
function paintFrame(n) {
    const out = [`${ESC}s`]; // save cursor
    for (let row = 1; row <= ROWS; row++) {
        const color = COLORS[(n + row) % COLORS.length];
        const bg = 40 + ((n + row) % 8);
        out.push(`${ESC}${row};1H`); // absolute position
        out.push(`${ESC}2K`); // clear line
        out.push(`${ESC}${color};${bg}m`);
        const cell = `${String.fromCharCode(0x2580 + ((n + row) % 16))}`;
        out.push(cell.repeat(Math.max(1, Math.min(COLS - 1, 120))));
        out.push(`${ESC}0m`);
    }
    out.push(`${ESC}u`); // restore cursor
    return out.join("");
}

/** Many tiny writes: the progress-bar/spinner pattern. Minimal bytes, maximal
 *  write COUNT — the shape that multiplies per-write overhead (WS frames,
 *  event dispatch) rather than byte volume. */
function spinnerFrame(n) {
    const glyph = "|/-\\"[n % 4];
    const pct = n % 101;
    const bar = "#".repeat(Math.floor(pct / 3)).padEnd(34, ".");
    return `\r${ESC}${COLORS[n % COLORS.length]}m${glyph} [${bar}] ${String(pct).padStart(3)}%${ESC}0m`;
}

// Rotation period for `mixed` mode, in wall-clock ms — see `frameFor` below
// for why this has to be TIME, not a frame count.
const MIXED_ROTATE_MS = 2000;
// The concrete load shapes `mixed` cycles through — deliberately NOT
// `VALID_MODES` (which also contains `"mixed"` itself, for CLI validation).
// `% VALID_MODES.length` was briefly tried and is a real bug, not a style
// choice: its length is 4, so one rotation index in four selects `"mixed"`,
// which recurses into `frameFor` with the SAME `elapsedMs` it was just
// called with — landing on index 3 again, forever. Caught by actually
// running the rotation with debug counters instead of trusting the diff;
// it manifests as a synchronous infinite recursion (stack overflow), not a
// subtle imbalance.
const MIXED_ROTATION_MODES = ["text", "paint", "spinner"];

function frameFor(mode, n, elapsedMs) {
    switch (mode) {
        case "text":
            return textFrame(n);
        case "paint":
            return paintFrame(n);
        case "spinner":
            return spinnerFrame(n);
        default: {
            // Codex P2 on PR #3286: this used to rotate every 120 FRAMES,
            // not every fixed time interval — and frame cost varies by two
            // orders of magnitude across modes (a spinner frame is ~20
            // bytes; a paint frame redraws the whole terminal). At the
            // default auto-paced rate, 120 spinner frames complete in well
            // under a millisecond while 120 paint frames take tens of
            // milliseconds, so "mixed" spent almost no time actually
            // applying spinner-shaped load — directly contradicting this
            // function's own former doc comment, which promised each class
            // "a sustained run" ("a second of each"). Rotating on elapsed
            // wall-clock time instead gives every class the same real
            // duration regardless of how cheap or expensive its frames are.
            const modeIdx = Math.floor(elapsedMs / MIXED_ROTATE_MS) % MIXED_ROTATION_MODES.length;
            return frameFor(MIXED_ROTATION_MODES[modeIdx], n, elapsedMs);
        }
    }
}

async function main() {
    const tag = LABEL ? ` label=${LABEL}` : "";
    await write(
        `pane-load: mode=${MODE} secs=${SECS} cols=${COLS}${tag}\n` +
            `pane-load: put your cursor in ANOTHER pane and type continuously once the flood starts.\n`
    );
    for (let i = COUNTDOWN; i > 0; i--) {
        await write(`pane-load: starting in ${i}...\n`);
        await sleep(1000);
    }
    await write(`pane-load: FLOOD START${tag}\n`);

    const started = Date.now();
    const deadline = started + SECS * 1000;
    let bytes = 0;
    let writes = 0;
    let n = 0;

    const maxBytes = MAX_MB * 1024 * 1024;
    // AUTO-PACING, and the reason it is the default: uncapped, these modes
    // sustain hundreds of MB/s, so the byte budget is spent in a fraction of a
    // second and the "flood" is over before a human can move their hands. The
    // point of this script is a window long enough to TYPE IN, so by default
    // spread the whole budget evenly across the requested duration —
    // 64 MB / 10 s ≈ 6.4 MB/s, a heavy-but-real build-log rate, sustained.
    // An explicit --throttle-ms or --unpaced opts out.
    const paced = !UNPACED && !THROTTLE_GIVEN;
    const targetBytesPerMs = maxBytes / (SECS * 1000);
    let cappedEarly = false;
    while (Date.now() < deadline) {
        if (bytes >= maxBytes) {
            cappedEarly = true;
            break;
        }
        const frame = frameFor(MODE, n++, Date.now() - started);
        const pending = write(frame);
        bytes += Buffer.byteLength(frame);
        writes++;
        if (pending) await pending;
        if (THROTTLE_MS > 0) await sleep(THROTTLE_MS);
        if (paced) {
            const owed = bytes / targetBytesPerMs - (Date.now() - started);
            if (owed > 1) await sleep(Math.min(owed, 50));
        }
    }

    const elapsed = (Date.now() - started) / 1000;
    const mb = bytes / (1024 * 1024);
    // `\n` first: spinner mode leaves the cursor mid-line, and paint mode
    // leaves colours set — reset both so the summary is readable.
    await write(
        `${ESC}0m\npane-load: FLOOD END${tag}\n` +
            `pane-load: ${mb.toFixed(1)} MB in ${elapsed.toFixed(1)}s ` +
            `(${(mb / elapsed).toFixed(2)} MB/s), ` +
            `${writes.toLocaleString()} writes (${Math.round(writes / elapsed).toLocaleString()}/s)\n` +
            // ReAgent P2 on PR #3286: cappedEarly was computed and then never
            // read — losing exactly the diagnostic this tool's own README
            // verification relied on to tell "ran the full window" apart from
            // "hit the disk cap and stopped short".
            (cappedEarly
                ? `pane-load: STOPPED EARLY at the ${MAX_MB} MB cap — the flood ran ` +
                  `${elapsed.toFixed(1)}s of the requested ${SECS}s. For a longer typing window, ` +
                  `throttle (--throttle-ms 1) rather than raising --max-mb: same stutter, far less disk.\n`
                : "")
    );
}

main().catch((err) => {
    process.stderr.write(`pane-load: ${err?.stack ?? String(err)}\n`);
    process.exit(1);
});
