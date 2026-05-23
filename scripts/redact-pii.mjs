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

writeFileSync(file, s);
const after = s.length;
console.log(`redacted ${file}: ${before} -> ${after} bytes`);
