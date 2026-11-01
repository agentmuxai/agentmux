// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reading source text for the ratchet gates (scripts/RATCHETS.md) without a parser:
// blank out what is not code, so a pattern or a brace count sees only code.

/**
 * `src` with comments, string literals and char literals replaced by spaces, newlines
 * kept, so every offset and line number is unchanged. Handles Rust: `//` and nested
 * block comments, `"..."` with escapes, raw strings `r"..."` / `r#"..."#`, and char
 * literals (a lifetime such as `'a` is not one and is kept).
 */
export function maskRust(src) {
    const out = [];
    const n = src.length;
    const blank = (from, to) => {
        for (let q = from; q < to; q++) out.push(src[q] === "\n" ? "\n" : " ");
    };
    let i = 0;
    while (i < n) {
        const c = src[i];
        const d = src[i + 1];
        if (c === "/" && d === "/") {
            let j = i;
            while (j < n && src[j] !== "\n") j++;
            blank(i, j);
            i = j;
        } else if (c === "/" && d === "*") {
            let depth = 1;
            let j = i + 2;
            while (j < n && depth > 0) {
                if (src[j] === "/" && src[j + 1] === "*") {
                    depth++;
                    j += 2;
                } else if (src[j] === "*" && src[j + 1] === "/") {
                    depth--;
                    j += 2;
                } else j++;
            }
            blank(i, j);
            i = j;
        } else if (c === "r" && (d === '"' || d === "#") && !/[A-Za-z0-9_]/.test(src[i - 1] ?? " ")) {
            let j = i + 1;
            let hashes = 0;
            while (src[j] === "#") {
                hashes++;
                j++;
            }
            if (src[j] !== '"') {
                out.push(c);
                i++;
                continue;
            }
            const close = '"' + "#".repeat(hashes);
            let k = src.indexOf(close, j + 1);
            k = k < 0 ? n : k + close.length;
            blank(i, k);
            i = k;
        } else if (c === '"') {
            let j = i + 1;
            while (j < n && src[j] !== '"') j += src[j] === "\\" ? 2 : 1;
            blank(i, Math.min(j + 1, n));
            i = j + 1;
        } else if (c === "'") {
            const m = /^'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.)|[^\\'\n])'/.exec(src.slice(i, i + 12));
            if (m) {
                blank(i, i + m[0].length);
                i += m[0].length;
            } else {
                out.push(c);
                i++;
            }
        } else {
            out.push(c);
            i++;
        }
    }
    return out.join("");
}

/**
 * Masked Rust text (see [`maskRust`]) with every `#[cfg(test)]` item blanked: a module
 * (its braces matched), or a one-line item such as a `use`. Line numbers are unchanged.
 */
export function blankCfgTest(masked) {
    const lines = masked.split("\n");
    for (let i = 0; i < lines.length; i++) {
        if (!/^\s*#\[cfg\(test\)\]/.test(lines[i])) continue;
        let j = i;
        // Further attributes, then the item.
        while (j + 1 < lines.length && /^\s*#\[/.test(lines[j + 1])) j++;
        j++;
        let depth = 0;
        let opened = false;
        for (; j < lines.length; j++) {
            for (const ch of lines[j]) {
                if (ch === "{") {
                    depth++;
                    opened = true;
                } else if (ch === "}") depth--;
            }
            if (opened && depth <= 0) break;
            if (!opened && lines[j].trim().endsWith(";")) break;
        }
        for (let k = i; k <= Math.min(j, lines.length - 1); k++) lines[k] = "";
        i = j;
    }
    return lines.join("\n");
}

/**
 * The repo paths of the module files that `path` declares under `#[cfg(test)]` as
 * `mod name;`: `name.rs` and `name/mod.rs` in the module's own folder. `masked` is
 * [`maskRust`] output for the file at `path`. Those files are test code however they
 * are named (`*_itest.rs`, say), so a gate that skips tests skips them.
 */
export function cfgTestModules(path, masked) {
    const slash = path.lastIndexOf("/");
    const dir = path.slice(0, slash + 1);
    const file = path.slice(slash + 1);
    // A module's children sit beside mod.rs, lib.rs and main.rs, or in a folder named for the file.
    const home = /^(mod|lib|main)\.rs$/.test(file) ? dir : `${dir}${file.replace(/\.rs$/, "")}/`;
    const out = [];
    const re = /#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/g;
    for (const m of masked.matchAll(re)) out.push(`${home}${m[1]}.rs`, `${home}${m[1]}/mod.rs`);
    return out;
}

/** Rust production code: comments and strings blanked, `#[cfg(test)]` items removed. */
export function rustCode(src) {
    return blankCfgTest(maskRust(src));
}
