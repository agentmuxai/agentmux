// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shape-based, best-effort secret redaction — the TypeScript half of
 * `SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md` §4.5's shared redactor.
 * `formatErrorReport` (`error-report.ts`) and every other copy action that
 * puts error or log text on the clipboard run through this before the text
 * is truncated or written.
 *
 * A faithful, deliberate port of `agentmux-common/src/redact.rs`'s
 * character-scanning algorithms (not a from-scratch regex rewrite) so the
 * two can't quietly diverge in what they consider a match — both are tested
 * against the same `docs/specs/fixtures/redaction-vectors.json`.
 *
 * This is shape-based and best-effort: a credential in an unrecognized form
 * gets through. Redact more, not less, when a shape is ambiguous.
 */

const SECRET_PREFIXES = [
    "github_pat_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "sk-",
    "xoxa-",
    "xoxb-",
    "xoxo-",
    "xoxp-",
    "xoxr-",
    "xoxs-",
    "AKIA",
];

const REDACTED_HEADERS = ["authorization", "proxy-authorization", "x-api-key", "api-key", "cookie", "set-cookie"];

const SENSITIVE_KEY_SUBSTRINGS = [
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "access_key",
    "private_key",
    "client_secret",
    "credential",
];

function isBodyChar(c: string): boolean {
    return /[A-Za-z0-9_-]/.test(c);
}

/** Blanks credential-prefixed tokens (`ghp_...`, `sk-...`, AWS access-key
 * ids, …). Shape-based: errs toward redacting a long token that merely
 * looks like a secret. */
function redactCredentialPrefixes(text: string): string {
    let out = "";
    let rest = text;
    for (;;) {
        let best: { at: number; prefix: string } | null = null;
        for (const p of SECRET_PREFIXES) {
            const at = rest.indexOf(p);
            if (at === -1) continue;
            if (best === null || at < best.at || (at === best.at && p.length > best.prefix.length)) {
                best = { at, prefix: p };
            }
        }
        if (best === null) {
            out += rest;
            break;
        }
        const { at, prefix } = best;
        const after = rest.slice(at + prefix.length);
        let bodyLen = 0;
        while (bodyLen < after.length && isBodyChar(after[bodyLen])) bodyLen++;
        out += rest.slice(0, at);
        out += bodyLen >= 16 ? "[redacted secret]" : rest.slice(at, at + prefix.length + bodyLen);
        rest = after.slice(bodyLen);
    }
    return out;
}

/** Blanks PEM private keys (`-----BEGIN ... PRIVATE KEY-----` blocks),
 * never a certificate. */
export function redactPrivateKeys(text: string): string {
    let out = text;
    let from = 0;
    for (;;) {
        const start = out.indexOf("-----BEGIN", from);
        if (start === -1) break;
        const nl = out.indexOf("\n", start);
        const headerEnd = nl === -1 ? out.length : nl;
        if (!out.slice(start, headerEnd).includes("PRIVATE KEY")) {
            from = headerEnd;
            continue;
        }
        const endRel = out.indexOf("-----END", start);
        let end: number;
        if (endRel === -1) {
            end = out.length;
        } else {
            const keyRel = out.indexOf("KEY-----", endRel);
            end = keyRel === -1 ? out.length : keyRel + "KEY-----".length;
        }
        out = out.slice(0, start) + "[redacted private key]" + out.slice(end);
        from = start + "[redacted private key]".length;
    }
    return out;
}

function preserveTrailingNewline(original: string, out: string): string {
    return original.endsWith("\n") && !out.endsWith("\n") ? out + "\n" : out;
}

/** Blanks `Authorization:` / `Proxy-Authorization:` / `x-api-key:` /
 * `api-key:` / `cookie:` / `set-cookie:` header values, line by line. */
function redactHeaders(text: string): string {
    const redacted = text
        .split("\n")
        .map((line) => {
            const m = line.match(/^(\s*)([^:\n]+):/);
            if (m) {
                const name = m[2].trim();
                if (REDACTED_HEADERS.some((h) => h === name.toLowerCase())) {
                    return `${m[1]}${name}: [redacted]`;
                }
            }
            return line;
        })
        .join("\n");
    return preserveTrailingNewline(text, redacted);
}

function isKeyChar(c: string): boolean {
    return /[A-Za-z0-9_-]/.test(c);
}

function isSensitiveKey(key: string): boolean {
    const lower = key.replace(/^['"]|['"]$/g, "").toLowerCase();
    return SENSITIVE_KEY_SUBSTRINGS.some((s) => lower.includes(s));
}

/** Blanks `key=value`, `key: value` and JSON `"key": "value"` pairs whose
 * key *contains* a sensitive substring, case-insensitive — deliberately
 * "contains", not "equals" (§4.5's literal wording). A key like `tokenizer`
 * is redacted by this rule even though the word alone in prose is harmless;
 * `redactCredentialPrefixes`'s own near-miss handling is what protects
 * ordinary prose, not this rule. */
function redactKeyValuePairs(text: string): string {
    const chars = Array.from(text);
    let out = "";
    let lastEnd = 0;
    let idx = 0;
    while (idx < chars.length) {
        const ch = chars[idx];
        if (ch !== "=" && ch !== ":") {
            idx++;
            continue;
        }
        let keyEndIdx = idx;
        if (keyEndIdx > 0 && (chars[keyEndIdx - 1] === '"' || chars[keyEndIdx - 1] === "'")) {
            keyEndIdx--;
        }
        let keyStartIdx = keyEndIdx;
        while (keyStartIdx > 0 && isKeyChar(chars[keyStartIdx - 1])) {
            keyStartIdx--;
        }
        if (keyStartIdx === keyEndIdx) {
            idx++;
            continue;
        }
        const key = chars.slice(keyStartIdx, keyEndIdx).join("");
        if (!isSensitiveKey(key)) {
            idx++;
            continue;
        }
        let v = idx + 1;
        while (v < chars.length && (chars[v] === " " || chars[v] === "\t")) v++;
        const valQuote = v < chars.length && (chars[v] === '"' || chars[v] === "'") ? chars[v] : null;
        const valStart = valQuote ? v + 1 : v;
        let w = valStart;
        if (valQuote) {
            while (w < chars.length && chars[w] !== valQuote) w++;
        } else {
            while (w < chars.length && ![" ", "\t", "\n", "\r", ",", "}", ")", ";"].includes(chars[w])) w++;
        }
        out += chars.slice(lastEnd, valStart).join("");
        out += "[redacted]";
        const skipTo = valQuote ? w + 1 : w;
        lastEnd = skipTo;
        if (valQuote && w < chars.length) out += chars[w];
        idx = skipTo;
    }
    out += chars.slice(lastEnd).join("");
    return out;
}

/** Blanks URL userinfo passwords: `scheme://user:password@host` keeps
 * `scheme://user:[redacted]@host`. */
function redactUrlUserinfo(text: string): string {
    let out = "";
    let lastEnd = 0;
    let searchFrom = 0;
    for (;;) {
        const schemeRel = text.indexOf("://", searchFrom);
        if (schemeRel === -1) break;
        const schemeEnd = schemeRel + 3;
        const stopMatch = text.slice(schemeEnd).search(/[/?#\s]/);
        const authorityEnd = stopMatch === -1 ? text.length : schemeEnd + stopMatch;
        const authority = text.slice(schemeEnd, authorityEnd);
        const atRel = authority.indexOf("@");
        if (atRel !== -1) {
            const atPos = schemeEnd + atRel;
            const userinfo = authority.slice(0, atRel);
            const colonRel = userinfo.indexOf(":");
            if (colonRel !== -1) {
                const colonPos = schemeEnd + colonRel;
                out += text.slice(lastEnd, colonPos + 1);
                out += "[redacted]";
                lastEnd = atPos;
            }
        }
        searchFrom = Math.max(authorityEnd, schemeEnd + 1);
        if (searchFrom > text.length) break;
    }
    out += text.slice(lastEnd);
    return out;
}

function isBase64UrlChar(c: string): boolean {
    return /[A-Za-z0-9_-]/.test(c);
}

/** Blanks JWTs: three base64url segments, the first starting `eyJ`. */
function redactJwts(text: string): string {
    let out = "";
    let lastEnd = 0;
    let searchFrom = 0;
    for (;;) {
        const start = text.indexOf("eyJ", searchFrom);
        if (start === -1) break;
        const prevChar = start > 0 ? text[start - 1] : "";
        if (prevChar && isBase64UrlChar(prevChar)) {
            searchFrom = start + 3;
            continue;
        }
        let seg1End = start;
        while (seg1End < text.length && isBase64UrlChar(text[seg1End])) seg1End++;
        if (text[seg1End] !== ".") {
            searchFrom = start + 3;
            continue;
        }
        const seg2Start = seg1End + 1;
        let seg2End = seg2Start;
        while (seg2End < text.length && isBase64UrlChar(text[seg2End])) seg2End++;
        if (seg2End === seg2Start || text[seg2End] !== ".") {
            searchFrom = start + 3;
            continue;
        }
        const seg3Start = seg2End + 1;
        let seg3End = seg3Start;
        while (seg3End < text.length && isBase64UrlChar(text[seg3End])) seg3End++;
        if (seg3End === seg3Start) {
            searchFrom = start + 3;
            continue;
        }
        out += text.slice(lastEnd, start);
        out += "[redacted]";
        lastEnd = seg3End;
        searchFrom = seg3End;
    }
    out += text.slice(lastEnd);
    return out;
}

function isBase64Char(c: string): boolean {
    return /[A-Za-z0-9+/=]/.test(c);
}

/** Blanks a 40-character base64 value on the same line as
 * `aws_secret_access_key` (any case). */
function redactAwsSecretKey(text: string): string {
    const redacted = text
        .split("\n")
        .map((line) => {
            if (!line.toLowerCase().includes("aws_secret_access_key")) return line;
            let i = 0;
            while (i < line.length) {
                if (!isBase64Char(line[i])) {
                    i++;
                    continue;
                }
                const runStart = i;
                let j = i;
                while (j < line.length && isBase64Char(line[j])) j++;
                if (j - runStart === 40) {
                    return line.slice(0, runStart) + "[redacted]" + line.slice(j);
                }
                i = j;
            }
            return line;
        })
        .join("\n");
    return preserveTrailingNewline(text, redacted);
}

/** The full redaction pipeline. Order matters — see `redact.rs`'s doc
 * comment; kept identical here so the two can't drift. */
export function redactSecrets(text: string): string {
    let out = redactCredentialPrefixes(text);
    out = redactPrivateKeys(out);
    out = redactHeaders(out);
    out = redactKeyValuePairs(out);
    out = redactUrlUserinfo(out);
    out = redactJwts(out);
    out = redactAwsSecretKey(out);
    return out;
}
