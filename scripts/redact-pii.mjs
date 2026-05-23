// One-shot PII redactor for docs/recovery/ transcripts.
// Run: node scripts/redact-pii.mjs <file>
import { readFileSync, writeFileSync } from "node:fs";

const file = process.argv[2];
if (!file) {
  console.error("usage: node scripts/redact-pii.mjs <file>");
  process.exit(2);
}

let s = readFileSync(file, "utf8");
const before = s.length;

// 1) Emails -> placeholder
s = s.replace(/[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}/g, "<redacted-email>");

// 2) AgentA-* identity slug -> placeholder
s = s.replace(/AgentA-[a-zA-Z0-9_-]+/g, "<redacted-user>");

// 3) Windows paths — both slash directions
//    Codex P1 on #990: the original only matched backslash paths, missing
//    forward-slash forms like `C:/Users/<user>/...` that show up wherever
//    a JS / Node tool normalized the path. Match BOTH separators in one
//    character class so the username segment is always replaced.
s = s.replace(/[Cc]:[\\/]+Users[\\/]+[^\\/\s]+[\\/]+/g, "~/");

// 4) Standalone username "area54" -> placeholder
s = s.replace(/\barea54\b/g, "<redacted-user>");

// 5) AppData/Local/Temp subpaths -> <tmp>
s = s.replace(/~\/AppData\/Local\/Temp\//g, "~/<tmp>/");
s = s.replace(/~[\\]+AppData[\\]+Local[\\]+Temp[\\]+/g, "~/<tmp>/");

// 6) Normalize Windows path separators to forward slash — and CRITICALLY,
//    break any `\<hex>` sequence that tailwind v4's CSS-escape regex
//    would interpret as a code point.
//
//    Background: tailwind v4 scans .md (and other) files looking for CSS
//    custom-property usages. Its regex `/\\([\dA-Fa-f]{1,6}…/` greedily
//    matches a backslash followed by 1-6 hex digits and passes the parse
//    to `String.fromCodePoint`. A path fragment like
//    `\d85077b2-8fd6-4397-…` (a UUID after a Windows path separator)
//    captures `\d85077` → 0xD85077 → invalid code point → THE WHOLE
//    PRODUCTION BUILD FAILS with "Invalid code point 14176375". Hit on
//    2026-05-23 — the Maks transcript brought down `task build:frontend`
//    (and every other agent's build) until this normalization landed.
//
//    Fix: replace single backslashes with forward slashes everywhere in
//    transcript text. Paths stay legible; the CSS-escape pattern is
//    structurally impossible afterwards.
s = s.replace(/\\(?!\\)/g, "/");

writeFileSync(file, s);
const after = s.length;
console.log(`redacted ${file}: ${before} -> ${after} bytes`);
