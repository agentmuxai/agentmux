# Codex App Server protocol snapshots

This directory holds versioned, generated protocol evidence for the exact Codex
CLI pin used by AgentMux. The committed files are copied without edits from:

```text
codex app-server generate-json-schema --out <temporary-directory>
```

Each version's `manifest.json` records SHA-256 hashes, the stable surface required
by AgentMux, and the methods found only when regenerating with `--experimental`.
AgentMux does not opt into the experimental API globally.

The CLI also generates one TypeScript file per protocol type. Those files are not
vendored because the v2 JSON Schema bundle is the language-neutral source of truth
and the controller is implemented in Rust. The manifest records the TypeScript
generation result so reviewers can reproduce it when the pin changes.

To update a snapshot:

1. bump all AgentMux Codex pins;
2. generate stable JSON Schema and TypeScript output with that exact CLI version;
3. copy the six files named by the prior manifest into the new version directory;
4. generate the experimental JSON Schema into a separate temporary directory and
   review the method-set difference;
5. update hashes and the stable/experimental inventory in `manifest.json`;
6. run `codex-app-server-schema-gate.test.ts`.
