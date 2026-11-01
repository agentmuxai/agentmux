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
 * Masked Rust text (see [`maskRust`]) with every `#[cfg(test)]` item blanked. The item
 * ends at the first of: its `{ ... }` block closing (a module, function, impl, struct), a
 * `;` (a `use`, a const), or a `,` (a struct field, an enum variant, a match arm, an
 * argument), each outside any nested brackets. `<` and `>` bracket generics only before an
 * `=` (in an expression they compare). An item nothing ends loses only its first line.
 * Line numbers are unchanged.
 */
export function blankCfgTest(masked) {
    const n = masked.length;
    const out = masked.split("");
    const attr = /#\[cfg\(test\)\]/g;
    let from = 0;
    for (let m = attr.exec(masked); m; m = attr.exec(masked)) {
        if (m.index < from) continue;
        let i = m.index + m[0].length;
        // Further attributes belong to the same item.
        for (;;) {
            const rest = /^\s*#\[/.exec(masked.slice(i, i + 200));
            if (!rest) break;
            let depth = 0;
            let j = i + rest[0].length - 1;
            for (; j < n; j++) {
                if (masked[j] === "[") depth++;
                else if (masked[j] === "]" && --depth === 0) break;
            }
            i = j + 1;
        }
        let paren = 0;
        let angle = 0;
        let block = 0;
        // Past an `=` the rest is an expression, where `<` and `>` compare instead of bracketing.
        let expr = false;
        let end = -1;
        for (let j = i; j < n; j++) {
            const ch = masked[j];
            const top = paren === 0 && block === 0;
            if (ch === "(" || ch === "[") paren++;
            else if (ch === ")" || ch === "]") {
                if (top) {
                    end = j;
                    break;
                }
                paren--;
            } else if (ch === "<" && top && !expr) angle++;
            else if (ch === ">" && top && !expr && masked[j - 1] !== "-" && angle > 0) angle--;
            else if (ch === "=" && top && angle === 0 && !/[=<>!]/.test(masked[j - 1]) && masked[j + 1] !== "=" && masked[j + 1] !== ">") expr = true;
            else if (ch === "{") block++;
            else if (ch === "}") {
                if (block === 0) {
                    end = j;
                    break;
                }
                block--;
                if (block === 0 && paren === 0) {
                    end = j + 1;
                    break;
                }
            } else if (top && angle === 0 && (ch === ";" || ch === ",")) {
                end = j + 1;
                break;
            }
        }
        // Nothing ended it: blank only the item's first line, so a misread can never take the rest of the file.
        if (end < 0) {
            const eol = masked.indexOf("\n", i);
            end = eol < 0 ? n : eol;
        }
        for (let k = m.index; k < end; k++) if (out[k] !== "\n") out[k] = " ";
        from = end;
    }
    return out.join("");
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

/**
 * `src` as TypeScript with comments blanked and the contents of string and template
 * literals blanked (the quotes stay), newlines kept, so every offset is unchanged.
 * Pattern matches on the result see only code; read the text of a match, such as a
 * module specifier, from `src` at the same offsets.
 */
export function maskTs(src) {
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
            let j = src.indexOf("*/", i + 2);
            j = j < 0 ? n : j + 2;
            blank(i, j);
            i = j;
        } else if (c === '"' || c === "'" || c === "`") {
            let j = i + 1;
            while (j < n && src[j] !== c && !(c !== "`" && src[j] === "\n")) j += src[j] === "\\" ? 2 : 1;
            out.push(c);
            blank(i + 1, Math.min(j, n));
            if (j < n && src[j] === c) out.push(c);
            i = j + 1;
        } else {
            out.push(c);
            i++;
        }
    }
    return out.join("");
}
