// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Fixture trees for scripts/gen-docs-index.test.mjs. Shared with
// make-goldens.mjs, which runs the ORIGINAL shell generator over the same
// trees to produce the *.golden files the tests compare against — so the
// expected outputs come from the implementation being replaced, not from a
// re-derivation of it.

import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

/** Environment for every git call and generator run in a fixture: no global
 *  or system config, no discovery of an enclosing repository. */
export function fixtureEnv(root) {
    return {
        ...process.env,
        GIT_CONFIG_NOSYSTEM: "1",
        GIT_CONFIG_GLOBAL: process.platform === "win32" ? "NUL" : "/dev/null",
        GIT_CEILING_DIRECTORIES: dirname(root),
        GIT_AUTHOR_NAME: "t",
        GIT_AUTHOR_EMAIL: "t@example.invalid",
        GIT_COMMITTER_NAME: "t",
        GIT_COMMITTER_EMAIL: "t@example.invalid",
        GITHUB_BASE_REF: "",
    };
}

export function git(root, ...args) {
    return execFileSync(
        "git",
        ["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main", ...args],
        { cwd: root, env: fixtureEnv(root), stdio: ["ignore", "pipe", "pipe"] }
    );
}

export function makeDir() {
    return mkdtempSync(join(tmpdir(), "gen-docs-index-"));
}

export function removeDir(root) {
    rmSync(root, { recursive: true, force: true });
}

/** Write raw bytes (strings are UTF-8). */
export function put(root, rel, content) {
    const p = join(root, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, typeof content === "string" ? Buffer.from(content, "utf8") : content);
}

export function initRepo(root) {
    git(root, "init", "-q");
}

export function commitAll(root, msg = "fixture") {
    git(root, "add", "-A");
    git(root, "commit", "-q", "--allow-empty", "-m", msg);
}

const CURATED = "# Specs index\n\nHand-written curated section.\n";

const filler = (n) => Array.from({ length: n }, (_, i) => `filler line ${i + 1}`).join("\n") + "\n";

/**
 * Every Status/title rule, the skip and exclusion rules, and ordering.
 * Committed except where a case needs the working tree to differ from HEAD.
 */
export function buildStatuses(root) {
    initRepo(root);
    put(root, "docs/specs/INDEX.md", CURATED);
    put(root, "docs/specs/README.md", "# Readme\n\n**Status:** living\n");
    const specs = {
        "impl.md": "# Implemented thing\n\n**Status:** implemented — #1\n",
        "act.md": "**Status:** active\n\nNo H1 here.\n",
        "prop.md": "# Mixed case\n\n**STATUS:** Proposed\n",
        "draft.md": "# Comma after word\n\n**Status:** Draft, pending review\n",
        "living.md": "# Tab after prefix\n\n**Status:**\tliving\n",
        "hist.md": "# Historical\n\n**Status:** historical\n",
        "sup.md": "# Superseded\n\n**Status:** superseded\n**Superseded-by:** impl.md\n",
        "none.md": "# No status line\n\nBody.\n",
        "late-status.md": "# Status after line 40\n" + filler(40) + "**Status:** implemented\n",
        "late-h1.md": "**Status:** draft\n" + filler(44) + "# Too late\n",
        "empty-h1.md": "#   \n# Second heading\n\n**Status:** draft\n",
        "pipe.md": "# A | B || C\n\n**Status:** proposed\n",
        "utf8.md": "# Café — naïve ✓\n\n**Status:** proposed\n",
        "phase.md": "# Punctuation joins words\n\n**Status:** active—Phase 0\n",
        "zeta.md": "# Non-canonical zeta\n\n**Status:** zeta\n",
        "alpha.md": "# Non-canonical alpha\n\n**Status:** Alpha.\n",
        "nbsp.md": "# NBSP is not a blank\n\n**Status:** x\u00a0draft\n",
        "dash-first.md": "# Dash first\n\n**Status:** — tbd\n",
        "blank-status.md": "# Blank status\n\n**Status:**\n",
        "h2-first.md": "## Not a title\n# Real title\n\n**Status:** active\n",
        "hash-no-space.md": "#NoSpace\n\n**Status:** active\n",
        "Upper.md": "# Uppercase sorts first\n\n**Status:** active\n",
        "with space.md": "# Space in name\n\n**Status:** active\n",
        ".hidden.md": "# Tracked dotfile\n\n**Status:** active\n",
        "second-status.md": "# First Status line wins\n\n**Status:** draft\n**Status:** implemented\n",
        "indented.md": "# Indented Status is not a Status line\n\n  **Status:** implemented\n",
        "deleted.md": "# Deleted from the working tree\n\n**Status:** active\n",
    };
    for (const [name, text] of Object.entries(specs)) put(root, `docs/specs/${name}`, text);
    put(root, "docs/specs/archive/old.md", "# Archived\n\n**Status:** historical\n");
    commitAll(root);
    // Working-tree-only changes.
    rmSync(join(root, "docs/specs/deleted.md"));
    put(root, "docs/specs/untracked.md", "# Untracked\n\n**Status:** active\n");
    put(root, "docs/specs/staged.md", "# Staged but not committed\n\n**Status:** implemented\n");
    git(root, "add", "docs/specs/staged.md");
}

/** INDEX.md with no marker: the generated section is appended after `---`. */
export function buildNoMarker(root) {
    initRepo(root);
    put(root, "docs/specs/INDEX.md", CURATED);
    put(root, "docs/specs/one.md", "# One\n\n**Status:** implemented\n");
    put(root, "docs/specs/two.md", "# Two\n\n**Status:** draft\n");
    commitAll(root);
}

/** INDEX.md with a marker, stale content after it, and the marker preceded by
 *  text on the same line: everything from that line on is replaced. */
export function buildMarker(root) {
    initRepo(root);
    put(
        root,
        "docs/specs/INDEX.md",
        CURATED +
            "\n---\n\nprefix text <!-- BEGIN GENERATED INDEX — edit scripts/gen-docs-index.sh, not this section -->\n" +
            "\nstale generated content\n\n<!-- END GENERATED INDEX -->\n\ntrailing text that is also replaced\n"
    );
    put(root, "docs/specs/one.md", "# One\n\n**Status:** implemented\n");
    commitAll(root);
}

/** An unresolved merge conflict on a spec: `git ls-files` lists it once per
 *  index stage. */
export function buildConflict(root) {
    initRepo(root);
    put(root, "docs/specs/INDEX.md", CURATED);
    put(root, "docs/specs/c.md", "# Base\n\n**Status:** draft\n");
    put(root, "docs/specs/other.md", "# Other\n\n**Status:** active\n");
    commitAll(root, "base");
    git(root, "checkout", "-q", "-b", "side");
    put(root, "docs/specs/c.md", "# Side\n\n**Status:** proposed\n");
    commitAll(root, "side");
    git(root, "checkout", "-q", "main");
    put(root, "docs/specs/c.md", "# Main\n\n**Status:** implemented\n");
    commitAll(root, "main");
    try {
        git(root, "merge", "-q", "side");
    } catch {
        // Expected: the conflict is the fixture.
    }
}

/** Not a git repository: the directory listing is used. */
export function buildNoGit(root) {
    put(root, "docs/specs/INDEX.md", CURATED);
    put(root, "docs/specs/b.md", "# B\n\n**Status:** draft\n");
    put(root, "docs/specs/A.md", "# A\n\n**Status:** implemented\n");
    put(root, "docs/specs/.dot.md", "# Dotfile, not matched by the glob\n\n**Status:** active\n");
    put(root, "docs/specs/archive/x.md", "# Archived\n\n**Status:** active\n");
    mkdirSync(join(root, "docs/specs/dir.md"), { recursive: true });
}

/** Inside a git repository that tracks none of these files (a tarball
 *  unpacked in an unrelated checkout): the directory listing is used. */
export function buildForeignRepo(root) {
    initRepo(root);
    put(root, "unrelated.txt", "tracked\n");
    commitAll(root);
    buildNoGit(root);
}

/** Every golden case: name → builder. The `root` passed to the generator is
 *  the fixture directory. */
export const GOLDEN_CASES = {
    statuses: buildStatuses,
    "no-marker": buildNoMarker,
    marker: buildMarker,
    conflict: buildConflict,
    "no-git": buildNoGit,
    "foreign-repo": buildForeignRepo,
};
