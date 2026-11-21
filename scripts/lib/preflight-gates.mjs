// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The gates `scripts/preflight.mjs` runs: the checks CI runs on a PR that need no build and no network, in the order
// CI lists them, each as the command CI runs. `scripts/preflight.test.mjs` reads the `docs` job of
// .github/workflows/ci-pr.yml and fails when a script it runs is in neither GATES nor NOT_RUN, so a gate added to CI
// cannot be forgotten here.

/** A gate that runs at once, alongside the others. `cmd` is the argv, run from the repository root. */
export const GATES = [
    { id: "doc-status", name: "doc status vocabulary", cmd: ["bash", "scripts/check-doc-status.sh"] },
    { id: "name-resolver-callers", name: "name-resolver callers", cmd: ["bash", "scripts/check-name-resolver-callers.sh"] },
    { id: "time-helpers", name: "no hand-rolled epoch time", cmd: ["node", "scripts/check-time-helpers.mjs"] },
    { id: "untyped-rpc-handlers", name: "no untyped RPC handlers", cmd: ["node", "scripts/check-untyped-rpc-handlers.mjs"] },
    { id: "comment-hygiene", name: "no review history in comments", cmd: ["node", "scripts/check-comment-hygiene.mjs"] },
    { id: "workdir-writes", name: "workdir writes through workdir_fs", cmd: ["node", "scripts/check-workdir-writes.mjs"] },
    { id: "audio-creators", name: "sounds through the sound service", cmd: ["node", "scripts/check-audio-creators.mjs"] },
    { id: "no-cef-version", name: "no CEF version in the overview", cmd: ["bash", "scripts/check-no-cef-version-in-overview.sh"] },
    { id: "product-text", name: "shipped text names no private setup", cmd: ["node", "scripts/check-product-text.mjs"] },
    { id: "rust-pin", name: "Rust version pinned everywhere", cmd: ["node", "scripts/check-rust-pin.mjs"] },
    { id: "test-support-off", name: "no build enables the test-only feature", cmd: ["node", "scripts/check-test-support-off.mjs"] },
    { id: "utf8", name: "tracked source is valid UTF-8", cmd: ["node", "scripts/check-utf8.mjs"] },
    { id: "docs-lifecycle", name: "docs lifecycle ratchet", cmd: ["node", "scripts/check-docs-lifecycle.mjs"] },
    { id: "docs-index", name: "specs index is current", cmd: ["node", "scripts/gen-docs-index.mjs", "--check"] },
    { id: "spec-citations", name: "spec citations resolve", cmd: ["bash", "scripts/check-spec-citations.sh"] },
    { id: "claude-md-references", name: "no reference to the removed CLAUDE.md", cmd: ["bash", "scripts/check-claude-md-references.sh"] },
    { id: "doc-links", name: "doc links resolve", cmd: ["node", "scripts/check-doc-links.mjs"] },
    { id: "docs-dir-map", name: "docs/ directory map", cmd: ["node", "scripts/check-docs-dir-map.mjs"] },
    { id: "new-spec-status", name: "a new spec does not say it is unbuilt", cmd: ["node", "scripts/check-new-spec-status.mjs"] },
    { id: "menu-positioning", name: "menu positioning", cmd: ["bash", "scripts/check-menu-positioning.sh"] },
    { id: "scrollbar-cursor", name: "scrollbar cursor", cmd: ["bash", "scripts/check-scrollbar-cursor.sh"] },
    { id: "no-transition-all", name: "no transition: all", cmd: ["bash", "scripts/check-no-transition-all.sh"] },
    { id: "motion-gates", name: "motion gates", cmd: ["node", "scripts/check-motion-gates.mjs"] },
    { id: "failure-writers", name: "failure writers", cmd: ["node", "scripts/check-failure-writers.mjs"] },
    { id: "transcript-scroll-writers", name: "transcript scroll writers", cmd: ["node", "scripts/check-transcript-scroll-writers.mjs"] },
    { id: "ui-primitives", name: "UI primitives ratchet", cmd: ["node", "scripts/check-ui-primitives.mjs"] },
    { id: "file-sizes", name: "file size ratchet", cmd: ["node", "scripts/check-file-sizes.mjs"] },
    { id: "reactive-coverage", name: "reactive coverage ratchet", cmd: ["node", "scripts/check-reactive-coverage.mjs"] },
    { id: "shared-paths", name: "shared paths are declared", cmd: ["node", "scripts/check-shared-paths.mjs"] },
    { id: "import-ratchets", name: "import ratchets", cmd: ["node", "scripts/check-import-ratchets.mjs"] },
    { id: "function-sizes", name: "function size ratchet", cmd: ["node", "scripts/check-function-sizes.mjs"] },
    { id: "srv-http-clients", name: "srv HTTP clients", cmd: ["node", "scripts/check-srv-http-clients.mjs"] },
    {
        id: "doc-references",
        name: "docs reference check",
        cmd: ["node", "scripts/check-doc-references.mjs", "--baseline", "scripts/doc-references.baseline"],
    },
    { id: "ligature-fonts", name: "ligature font gate", cmd: ["bash", "scripts/check-ligature-fonts.sh"] },
    { id: "bundled-tools", name: "bundled tools gate", cmd: ["bash", "scripts/check-bundled-tools.sh"] },
    { id: "taskfile-target-dir", name: "Taskfile target directory", cmd: ["bash", "scripts/check-taskfile-target-dir.sh"] },
    { id: "taskfile-dev-cargo-env", name: "Taskfile dev cargo environment", cmd: ["bash", "scripts/check-taskfile-dev-cargo-env.sh"] },
    { id: "hash-from-stdin", name: "hash files from stdin", cmd: ["bash", "scripts/check-hash-from-stdin.sh"] },
    { id: "input-handler-layout-reads", name: "input handlers read no layout", cmd: ["bash", "tools/lint/check-input-handler-layout-reads.sh"] },
    { id: "muxbus-credential-store", name: "muxbus credential store", cmd: ["bash", "scripts/check-muxbus-credential-store.sh"] },
];

/**
 * Gates that wait for the host's build slot and run one at a time, under a single ticket. `paths` says which changed
 * files make a gate worth its minutes: a gate runs when one of them starts with an entry, or with `--all`.
 */
export const HEAVY_GATES = [
    {
        id: "tsc",
        name: "tsc --noEmit",
        cmd: ["node", "node_modules/typescript/bin/tsc", "--noEmit", "-p", "tsconfig.citypecheck.json"],
        paths: ["frontend/", "test/", "tsconfig", "package.json", "package-lock.json"],
    },
    {
        id: "rpc-stubs",
        name: "RPC stubs are current",
        cmd: ["node", "scripts/gen-rpc-stubs.mjs", "--check"],
        paths: ["frontend/", "crates/", "scripts/gen-rpc-stubs.mjs"],
    },
    {
        id: "mux-events",
        name: "mux events are current",
        cmd: ["node", "scripts/gen-mux-events.mjs", "--check"],
        paths: ["frontend/", "crates/", "scripts/gen-mux-events.mjs"],
    },
];

/**
 * Scripts the `docs` job of ci-pr.yml runs that preflight does not, with the reason: the self-tests of build,
 * packaging and CI scripts, which only matter to a PR that edits the script they test.
 */
export const NOT_RUN = Object.fromEntries(
    [
        "scripts/cef-verify.test.sh",
        "scripts/cef-build/verify-glibc-floor.test.sh",
        "scripts/cef-build/verify-runtime-archive.test.sh",
        "scripts/vite-port.test.sh",
        "scripts/dev-vite.test.sh",
        "scripts/linux-package-deps.test.sh",
        "scripts/prune-dev-builds.test.sh",
        "scripts/ensure-ninja.test.sh",
        "scripts/cef-build/verify-cef-runtime-windows.test.sh",
        "scripts/verify-cef-framework-darwin.test.sh",
        "scripts/verify-srv-identity-darwin.test.sh",
        "scripts/verify-app-identity-darwin.test.sh",
        "scripts/macos-signing-scripts.test.sh",
        "scripts/agentmux-install-update.test.sh",
        "scripts/check-hash-from-stdin.test.sh",
        "scripts/check-anonymous-pull.test.sh",
        "scripts/ci-apt.test.sh",
        "scripts/ci-install-go-task.test.sh",
        "scripts/build-slot/build-slot.test.sh",
        "scripts/ci-nextest-run.test.sh",
        "scripts/check-taskfile-target-dir.test.sh",
        "scripts/check-taskfile-dev-cargo-env.test.sh",
    ].map((script) => [script, "self-test of a build, packaging or CI script; it only matters to a PR that edits that script, and CI runs it"]),
);

/** The Rust crates CI compiles with `cargo check --tests` on every host (the CEF-free ones). */
export const RUST_PACKAGES = [
    "agentmux-common",
    "agentmux-srv-core",
    "agentmux-srv",
    "agentmux-launcher",
    "agentmux-bashwrap",
    "agentmux-mcp",
    "agentmux-remote",
    "agentmux-procstats",
];
