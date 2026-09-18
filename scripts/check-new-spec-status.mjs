// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-new-spec-status.mjs — CI gate: a spec that ships WITH its own
// implementation must not still say it is unbuilt.
//
// THE RULE: if this PR adds a doc, and source files in the same PR cite that
// doc by name, the doc's `**Status:**` must not be `draft` or `proposed`.
//
// WHY THIS EXISTS
//
// Triaging the §5.6 reverse check (#3382) by hand across #3384, #3385 and
// #3386 turned up one cause behind most of it, over and over: the spec was
// added to the tree by its OWN implementing PR — which is `feedback_no_doc_
// only_prs` working exactly as designed, the doc riding along with the code —
// carrying a Status written before that PR merged, and nobody re-read the
// header on the way past. Three examples of what that leaves behind:
//
//   SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES  `Proposed`, while containing a
//                                          section titled "found during
//                                          implementation"
//   SPEC_AGENT_PICKER_TWO_TIER             "needs answers to the two decision
//                                          points", in the same commit that
//                                          added the section answering both
//   SPEC_INAPP_CLAUDE_OAUTH_LOGIN          `PROPOSED (implemented — see note
//                                          below)`, plus its own audit note
//                                          reading "Status field was never
//                                          updated" — six weeks earlier
//
// Every one of those was cheap to prevent and expensive to find later. This
// gate is the prevention; the sweep's reverse check stays as the backstop for
// specs that went stale some other way.
//
// WHY "SOURCE CITES IT", NOT "PR TOUCHES SOURCE"
//
// The weaker signal — this PR adds a spec and also touches Rust/TypeScript —
// is what `docs-status-origin-survey.mjs` uses to RANK the existing backlog,
// and it is deliberately not what this gate uses, because it has real false
// positives. `SPEC_DECISION_PROMPT_2026_04_24` was introduced by
// "feat(statusbar): always-show connection widget" (#553), a PR with nothing
// to do with it; that spec was a genuine proposal and `Draft` was the honest
// status. Failing that PR would teach people to route around this check.
//
// Requiring the source to name the doc separates the two cleanly. Replayed
// against the real commits (each run against its own parent):
//
//   #3090 zoom               proposed, 13 citing sources  -> caught
//   #2932 armory             proposed, 14                 -> caught
//   #1301 sounds             draft,    12                 -> caught
//   #1301 tool-call tones    draft,     4                 -> caught
//   #553  decision-prompt    (added no doc in that commit) -> nothing to check
//
// A PR can still legitimately add a proposal alongside unrelated code. It
// cannot legitimately add a proposal that its own new code implements.
//
// ONE KNOWN MISS, and why it is acceptable. Replaying #1011 (the two-tier
// picker) shows 13 citing sources but passes, because its Status read
// "Design analysis — ..." — first word `design`, which is not `draft` or
// `proposed` and so is not in UNBUILT. That is a pre-vocabulary doc: the
// closed `**Status:**` enum did not exist in May. `check-doc-status.sh` now
// runs on added files too and rejects anything outside the enum, so a doc
// added today cannot reach this gate saying `design`. The two gates compose;
// widening UNBUILT to "anything not implemented" would instead fire on
// `historical`, `superseded`, `living`, `analysis` and `retro`, all of which
// are legitimate things for a new doc to be.
//
// SCOPED TO ADDED DOCS, which is the whole point: this never fires on a doc
// that already existed, so it cannot resurrect the backlog it exists to stop
// growing. Editing a stale spec is the reverse check's job, not this one's.
//
// Usage:
//   node scripts/check-new-spec-status.mjs          # added-vs-origin/main
//   node scripts/check-new-spec-status.mjs --list   # report, never fails
import { execFileSync } from "node:child_process";

const args = process.argv.slice(2);
const LIST_ONLY = args.includes("--list");

const UNBUILT = new Set(["draft", "proposed"]);
const SOURCE_EXTS = new Set(["rs", "ts", "tsx", "mjs", "cjs", "js"]);

function git(...a) {
    return execFileSync("git", a, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
}

/** First word of the first `**Status:**` line, lowercased, letters only. */
export function statusOf(text) {
    const m = text.match(/^\*\*Status:\*\*\s*(.+)$/m);
    if (!m) return null;
    const word = m[1].trim().split(/[\s,.;:—–-]+/)[0] ?? "";
    const letters = word.replace(/[^A-Za-z]/g, "").toLowerCase();
    return letters || null;
}

/** The name source files would cite a doc by: its basename without `.md`. */
export function docName(path) {
    return path.split("/").pop().replace(/\.md$/, "");
}

export function isSourcePath(path) {
    return SOURCE_EXTS.has(path.split(".").pop()) && !path.startsWith("docs/");
}

/**
 * The decision, as a pure function so it can be tested without git.
 *
 * `citingSources` is the list of changed source paths whose content mentions
 * `docName(doc)`. A doc is a violation when it is newly added, says it is
 * unbuilt, and its own PR's source names it.
 */
export function isViolation({ status, citingSources }) {
    if (status === null) return false; // no Status line is a different gate's problem
    if (!UNBUILT.has(status)) return false;
    return citingSources.length > 0;
}

function baseRef() {
    const base = process.env.GITHUB_BASE_REF || "main";
    try {
        git("rev-parse", "--verify", "--quiet", `origin/${base}`);
        return `origin/${base}`;
    } catch {
        return base;
    }
}

function main() {
    let mergeBase;
    try {
        mergeBase = git("merge-base", baseRef(), "HEAD").trim();
    } catch {
        console.error("check-new-spec-status: no merge base; nothing to check.");
        return 0;
    }

    const status = git("diff", "--name-status", "-M", `${mergeBase}...HEAD`)
        .split("\n")
        .filter(Boolean)
        .map((l) => l.split("\t"));

    // Renames are NOT additions. #2920 moved 60-odd specs from `specs/` into
    // `docs/specs/` in one commit; treating those as new would fail a PR for
    // statuses it never wrote.
    const addedDocs = status
        .filter(([k, p]) => k[0] === "A" && p.endsWith(".md") && p.startsWith("docs/"))
        .map(([, p]) => p);
    if (addedDocs.length === 0) {
        console.error("check-new-spec-status: no new docs in this change.");
        return 0;
    }

    const changedSources = status
        .filter(([k]) => "AMR".includes(k[0]))
        .map((parts) => parts[parts.length - 1])
        .filter(isSourcePath);

    // Read each source once, not once per doc.
    const sourceText = new Map();
    for (const f of changedSources) {
        try {
            sourceText.set(f, git("show", `HEAD:${f}`));
        } catch {
            /* deleted or unreadable in HEAD */
        }
    }

    const violations = [];
    for (const doc of addedDocs) {
        let text;
        try {
            text = git("show", `HEAD:${doc}`);
        } catch {
            continue;
        }
        const name = docName(doc);
        const citingSources = [...sourceText]
            .filter(([, body]) => body.includes(name))
            .map(([f]) => f);
        const status = statusOf(text);
        if (LIST_ONLY) {
            console.error(
                `  ${doc}  status=${status ?? "(none)"}  cited by ${citingSources.length} changed source file(s)`,
            );
            continue;
        }
        if (isViolation({ status, citingSources })) violations.push({ doc, status, citingSources });
    }

    if (violations.length) {
        console.error("");
        console.error("A spec in this PR is implemented by this PR and still says it is not:");
        console.error("");
        for (const { doc, status, citingSources } of violations) {
            console.error(`  ${doc}`);
            console.error(`      **Status:** ${status}`);
            for (const f of citingSources.slice(0, 5)) console.error(`      cited by  ${f}`);
            if (citingSources.length > 5) {
                console.error(`      ...and ${citingSources.length - 5} more`);
            }
            console.error("");
        }
        console.error("Give it a Status that matches what this PR does — `implemented`, citing");
        console.error("this PR, or `active` saying what shipped and what is left. The vocabulary");
        console.error("is in docs/specs/README.md.");
        console.error("");
        console.error("If the spec really is still a proposal and the naming overlap is a");
        console.error("coincidence, rename the doc or drop the citation — a source file naming a");
        console.error("spec is how the rest of this repo's tooling decides the spec is real.");
        console.error("");
    }

    console.error(
        `check-new-spec-status: ${addedDocs.length} new doc(s), ` +
            `${changedSources.length} changed source file(s), ${violations.length} violation(s)`,
    );
    return violations.length ? 1 : 0;
}

if (import.meta.url === `file://${process.argv[1].replace(/\\/g, "/")}`) {
    process.exit(main());
}
