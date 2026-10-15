// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The line diff Memory's history views show, for Personal and Global
//! Memory alike (`native_memory_handlers`, `app_api::global_memory`).

/// Cap on the LCS table's cell COUNT (from_lines.len() * to_lines.len()),
/// not on either side's line count independently — reagent P1: the
/// original per-side-only cap (20,000 lines each) bounded neither
/// dimension against the other, so two files each under that cap could
/// still produce a ~3.2GB `usize` table (20_000 * 20_000 * 8 bytes) in the
/// shared agentmux-srv process. 4,000,000 cells keeps the table under
/// ~32MB (`* size_of::<usize>()`) regardless of how the two side lengths
/// are distributed — generous for memory files (markdown notes, not logs)
/// while still bounded for any combination of sizes.
const MAX_DIFF_CELLS: usize = 4_000_000;

/// A minimal unified-diff-style line comparison: longest-common-subsequence
/// based, output lines prefixed `"  "` (context), `"- "` (removed, `from`
/// only), or `"+ "` (added, `to` only). No `@@` hunk headers or context
/// trimming in v1 — every line is included, which is fine for memory files.
pub(crate) fn line_diff(from: &str, to: &str) -> String {
    let from_lines: Vec<&str> = from.lines().collect();
    let to_lines: Vec<&str> = to.lines().collect();

    if from_lines.len().saturating_mul(to_lines.len()) > MAX_DIFF_CELLS {
        return format!(
            "(diff omitted: {} x {} lines exceeds the {MAX_DIFF_CELLS}-cell comparison cap)",
            from_lines.len(),
            to_lines.len(),
        );
    }

    let n = from_lines.len();
    let m = to_lines.len();
    let cols = m + 1;
    // A single flat allocation, not `n + 1` separate `Vec<usize>` rows —
    // reagent P2: MAX_DIFF_CELLS bounds the cell COUNT (n * m), but a
    // maximally lopsided diff (e.g. ~4,000,000 short lines vs. 1 line)
    // stays under that cap while `vec![vec![...]; n + 1]` would still
    // perform ~4,000,001 individual heap allocations — one per row — whose
    // allocator overhead and allocation-count latency dwarf the actual
    // cell-data cost the cap was meant to bound. lcs[i][j] (length of the
    // longest common subsequence of from_lines[i..] and to_lines[j..])
    // lives at flat index i * cols + j.
    let mut lcs = vec![0usize; (n + 1) * cols];
    let idx = |i: usize, j: usize| i * cols + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[idx(i, j)] = if from_lines[i] == to_lines[j] {
                lcs[idx(i + 1, j + 1)] + 1
            } else {
                lcs[idx(i + 1, j)].max(lcs[idx(i, j + 1)])
            };
        }
    }

    let mut out = String::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if from_lines[i] == to_lines[j] {
            out.push_str("  ");
            out.push_str(from_lines[i]);
            out.push('\n');
            i += 1;
            j += 1;
        } else if lcs[idx(i + 1, j)] >= lcs[idx(i, j + 1)] {
            out.push_str("- ");
            out.push_str(from_lines[i]);
            out.push('\n');
            i += 1;
        } else {
            out.push_str("+ ");
            out.push_str(to_lines[j]);
            out.push('\n');
            j += 1;
        }
    }
    while i < n {
        out.push_str("- ");
        out.push_str(from_lines[i]);
        out.push('\n');
        i += 1;
    }
    while j < m {
        out.push_str("+ ");
        out.push_str(to_lines[j]);
        out.push('\n');
        j += 1;
    }
    out
}
