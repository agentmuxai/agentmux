# Version drift upgrades (2026-10-01 report) and provider harness tests

**Status:** active. Phase 1a ships with this spec. Later phases each land as their own PR, and §7 tracks them.
**Date:** 2026-10-01
**Owner:** Manoz
**Source:** `a5af/shared-infrastructure` `provider-reporter`, "AgentMux Version Drift Report - Oct 01, 2026 - 6 CLI(s) behind, 15 version(s) behind".
**Builds on:** `SPEC_PROVIDER_CLI_VERSION_UPGRADE_2026_09_06.md` (the last CLI round, whose pin-location table this spec reuses) and `SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md` (the general process).

## 1. Goal

1. Bring every item that the 2026-10-01 drift report marks `behind` up to date, in phases ordered by risk. One PR per phase. Each breaking library upgrade gets its own PR.
2. After the CLI bumps, run a thorough basic-functionality pass over every provider harness, not just Claude, and record the results with evidence (§6).

## 2. Findings

### 2.1 Provider CLIs

These were re-checked against npm on 2026-10-01 at about 21:00 UTC. That's later than the report, so several "latest" values have moved since.

| Provider | Package | Pinned on main | Report latest | npm latest now | Action |
|---|---|---|---|---|---|
| claude | `@anthropic-ai/claude-code` | 2.1.285 | 2.1.286 | 2.1.287 | bump (phase 1a) |
| gemini | `@google/gemini-cli` | 0.60.0 | 0.62.0 | 0.62.0 | bump (phase 1a) |
| qwen | `@qwen-code/qwen-code` | 0.24.0 | 0.24.7 | 0.24.7 | bump (phase 1a) |
| openclaw | `openclaw` | 2026.9.4 | 2026.9.7 | 2026.9.7 | bump (phase 1a) |
| copilot | `@github/copilot` | 1.0.85 | 1.0.90 | 1.0.91 | bump (phase 1a) |
| codex | `@openai/codex` | 0.154.0 | 0.159.3 | 0.160.0 | bump (phase 1b, §4.2) |
| pi | `@mariozechner/pi-coding-agent` | 0.73.1 | 0.73.1 | 0.73.1 | none |
| muxcode, antigravity | — | 0.1.0 / 1.0.0 | lookup-failed | 404 | none. These aren't published on npm, as explained in the 09-06 spec §3. |
| kimi | (pip) | — | unmonitored | — | none |

The model catalog section is all `current`, so it needs no change.

### 2.2 Platform, CEF and bundled tools

| Item | Pinned | Latest | Where | Phase |
|---|---|---|---|---|
| cef-rs binding (`cef-dll-sys`) | 154.2.0+154.0.28 | 154.3.0+154.0.32 | `Cargo.toml`, patched from `agentmuxai/cef-rs` | 2 |
| Node.js (desktop) | 24.11.0 | 24.21.0 | `.nvmrc`, `package.json` `engines` | 2 |
| jq (bundled) | 1.7.1 | 1.8.2 | `scripts/package-portable.sh` | 2 |
| ripgrep (bundled) | 14.1.1 | 15.2.0 | `scripts/package-portable.sh` | 2 |
| Rust toolchain | unpinned (stable) | 1.99.0 | no `rust-toolchain.toml` | 2 (decision, §4.3) |
| CEF runtime (all 3 platforms), CEF milestone | 154.0.8037.58 | same | — | none |
| Lambda runtimes | — | — | AWS | none. The report marks them all `current` (they're within support). |

### 2.3 Key libraries

The report flags the first three as security-relevant, so any gap counts.

| Library | Pinned | Latest | Kind | Phase |
|---|---|---|---|---|
| rustls | 0.23.40 | 0.23.45 | patch (security) | 2 |
| ed25519-dalek (jekt signing) | 2.2.0 | 3.0.0 | major (security) | 3 |
| marked (agent markdown) | 16.4.2 | 18.0.14 | major (security) | 3 |
| reqwest | 0.12.28 | 0.13.5 | breaking 0.x | 4 |
| rusqlite (bundled SQLite) | 0.31.0 | 0.40.2 | breaking 0.x | 4 |
| keyring (OS credential store) | 2.3.3 | 4.2.0 | major | 4 |
| katex | 0.16.47 | 0.18.10 | breaking 0.x | 4 |
| shiki | 3.23.0 | 4.5.0 | major | 4 |
| mermaid | 11.16.1 | 12.0.0 | major | 4 |
| vite | 6.4.3 | 8.3.2 | major (two majors) | 4 |
| typescript | 5.9.3 | 7.0.2 | major (two majors) | 4 |

## 3. Phasing

| Phase | Scope | Risk | PR shape |
|---|---|---|---|
| 1a | claude, gemini, qwen, openclaw, copilot pins | low: version strings, plus flag re-verification (§4.1) | this PR, together with this spec |
| 1b | codex pin | medium: versioned app-server schema, replay fixtures, default model | one PR |
| 2 | patch/minor platform items: cef-dll-sys, Node 24.21, jq, ripgrep, rustls, Rust toolchain decision | low to medium | one PR, or two if the cef-rs fork rebase is non-trivial |
| 3 | ed25519-dalek 3, marked 18 | medium: wire format, sanitising | one PR each |
| 4 | reqwest, rusqlite, keyring, katex, shiki, mermaid, vite, typescript | medium to high | one PR each, in the order of §4.5 |
| 5 | provider harness test pass (§6) | — | a results report, plus fix PRs for whatever fails |

Phase 5 runs right after phase 1b, as soon as all the CLI pins are new. It doesn't wait for phases 2–4: the CLI bumps are what change harness behaviour, and testing them early keeps a failure attributable to a single bump.

## 4. Per-phase requirements

### 4.1 Phase 1a: five CLI pins

Pin locations, enforced by `frontend/app/view/agent/providers/pin-consistency.test.ts`:

| Location | claude | gemini | qwen | openclaw | copilot |
|---|---|---|---|---|---|
| `frontend/app/view/agent/providers/catalog.ts` `pinnedVersion` | ✅ | ✅ | ✅ | ✅ | ✅ |
| `crates/srv/src/backend/providers.rs` `pinned_version` | ✅ | ✅ | ✅ | ✅ | ✅ |
| `.github/workflows/container-image.yml` `claude_version` | ✅ | — | — | — | — |
| `docker/Dockerfile.agent-agentmux` `ARG CLAUDE_VERSION` | ✅ | — | — | — | — |

A version string isn't the whole contract. `crates/srv/src/backend/blockcontroller/subprocess/argv.rs` records, for each provider, which flags were "checked against the pinned CLI". For each bumped provider, install the new version into a scratch prefix (`npm install --prefix <tmp> <pkg>@<new>`) and confirm that every flag and subcommand AgentMux passes still appears in `--help`. Update the "checked against" comments to the new version. Test fixtures that only illustrate a version string (`cli-notice.test.ts`, `version-drift.test.ts`) stay as they are, because they test parsing, not the pin.

**Done for this PR on 2026-10-01.** I installed the old and new version of each CLI side by side and compared their help output for every flag and subcommand that `providers.rs`, `catalog.ts`, `argv.rs` and `buildRuntimeArgs.ts` pass:

- **claude 2.1.285 → 2.1.287:** 15 flags and 2 auth subcommands all present. `--max-turns` is hidden from help in both versions. The only removal is `--client-data-url`, which AgentMux doesn't use.
- **gemini 0.60.0 → 0.62.0:** `--output-format --yolo -p -r --approval-mode --policy -m --model` all present, and the help text is otherwise identical.
- **qwen 0.24.0 → 0.24.7:** `--output-format --yolo -p --model` all present.
- **copilot 1.0.85 → 1.0.91:** `--acp --model`, and `auth login`, all present.
- **openclaw 2026.9.4 → 2026.9.7:** `acp`, `models auth list|login --provider` all present. This had to be checked under Node 24.21. See the finding below.

The `GEMINI_DENY_ALL_TOOLS_POLICY` note in `argv.rs` says it was verified "by capturing its request" at 0.60.0. That needs a live request capture at 0.62.0, not just a help check, so the comment keeps 0.60.0 until phase 5's B4 check for gemini repeats the capture.

**Finding: OpenClaw needs Node ≥ 24.16 (both versions).** Its preinstall script refuses Node 22, and the CLI itself exits on Node 22.22.2 with "node:sqlite truncates TEXT at embedded NUL (nodejs/node#61954); use 24.16+/26.1+". AgentMux's `NODE_PREREQ` checks only that `node` exists, not its version. So on any machine with Node 22, including Area54 today, opening an OpenClaw agent fails at install. Phase 2's Node bump (desktop 24.21.0) and a minimum-version check for each provider's prereqs are the fixes. Phase 5 records the user-visible failure as OpenClaw B1.

### 4.2 Phase 1b: codex

Follow `schema/providers/codex/app-server/README.md`:
1. Bump the pins.
2. Run `codex app-server generate-json-schema` at the new version into `schema/providers/codex/app-server/<new>/`.
3. Review the stable and experimental method-set diff, update `manifest.json`, and run `codex-app-server-schema-gate.test.ts`.

After that:
- Re-capture the five replay fixtures under `frontend/test/fixtures/providers/codex/<new>/`, as for 0.154.0: normal, command, file-change, resume, docker-resume.
- Re-verify the four places where `app_server_protocol.rs` / `argv.rs` cite 0.154.0 behaviour.
- Re-verify `CODEX_DEFAULT_MODEL` for ChatGPT-account auth, as the 09-06 spec §4.1 requires.

The old version's directory stays until the new one has been verified live.

### 4.3 Phase 2 notes

- **cef-dll-sys:** rebase the `agentmuxai/cef-rs` patch onto the 154.3.0 tag and bump the `rev`. The runtime pin (`scripts/cef-build/windows-runtime-pin.sh`) doesn't change, because the binding moves within milestone 154.
- **Node:** bump `.nvmrc` and the `engines` floor to 24.21.0. CI's `node-version: 24` already tracks the latest 24.x.
- **jq / ripgrep:** bump the URLs and expected hashes in `package-portable.sh`. ripgrep 15 is a major: check that the flags agents use (`rg --json`, `--glob`, `-n`) are unchanged.
- **Rust toolchain:** the decision is whether to add `rust-toolchain.toml` pinned to 1.99.0. A pin gives reproducible local and CI builds, at the cost of a bump PR per release. My recommendation is yes, with the drift reporter keeping it fresh.

### 4.4 Phase 3: security-flagged majors

- **ed25519-dalek 3:** jekt signatures must stay byte-compatible across a mixed-version fleet. The machines upgrade at different times: right now Area54 and narko run 0.59.x, and charlie runs 0.58.2. The PR has to include a cross-version test: verify a signature made by 2.x with 3.x, and the other way round, for the exact key and signature encodings used by `wan_verify`/`wan_publish`.
- **marked 18:** agent markdown rendering goes through `marked` and then DOMPurify. Re-run the markdown render and sanitise suites (`app/element/markdown*.test.*`). Diff the rendered HTML for a corpus of real transcripts before and after. Pay particular attention to raw-HTML passthrough and link handling.

### 4.5 Phase 4: breaking upgrades, one PR each, in this order

1. **reqwest 0.13**: used by `srv`, `cef`, `mcp` and `bashwrap`. Mostly mechanical, but TLS feature names change.
2. **rusqlite 0.40**: brings a newer bundled SQLite. All schema migrations must apply to a copy of a real data directory, and downgrading must still open the database.
3. **keyring 4**: AgentMux's sign-in to its cloud (MuxBus) is stored in fixed-name keychain entries. Existing entries must still read after the upgrade. This is also the natural point to fix the known gap where `AGENTMUX_ISOLATED_AUTH` doesn't scope the MuxBus keychain entries (found 2026-09-29, being filed separately). Keep that fix a separate commit or PR.
4. **katex 0.18, shiki 4, mermaid 12**: output-rendering libraries. Compare screenshots of math, code and diagram rendering in the agent pane.
5. **vite 8**: two majors. Dev server, HMR, and the `task package` build output must all still work.
6. **typescript 7**: TS 7 is the native (Go) compiler. Confirm that `tsc --noEmit`, editor tooling, and every package that depends on the TypeScript API, such as the eslint parser and ts-rs consumers, still work. This one goes last and may be deferred.

## 5. Shared verification for every upgrade PR

- `npx vitest run` (full), `npx tsc --noEmit -p .`, and `cargo test --workspace` for any Rust change.
- `task package`, then launch the portable built from the branch, open an agent, and send a prompt. That's the smoke test.
- A changeset (`scripts/changeset.sh patch "<what changed>"`). Never edit version files by hand.

## 6. Provider harness test pass (phase 5)

Run this in a fresh `task package` portable built from `main` once phases 1a and 1b have merged. Run it on its own channel, and **do not sign in to the AgentMux cloud from the test instance**: the MuxBus keychain entries are machine-wide, so that sign-in would replace the main instance's session. Provider logins are fine; they use AgentMux's own per-channel identity store.

The providers: claude, codex, gemini, qwen, openclaw, copilot, pi, muxcode, antigravity and kimi. For each one, run every check below and record pass, fail or n/a, with evidence: a log line, a screenshot, or a transcript excerpt.

| # | Check | Pass means |
|---|---|---|
| B1 | Install | The first open installs exactly the pinned version into `shared/cli/<provider>/<version>/`, and `<cli> --version` matches the pin. |
| B2 | Auth state | Signed out, the pane shows a clear sign-in path rather than a hang or a raw error. Signed in, it starts without prompting. |
| B3 | First turn | A short prompt streams a reply, the turn ends, and the translator logs no unknown or unhandled event types. |
| B4 | Tool use | A read-file call and a shell call both run. The approval prompt appears where the provider requires one and is honoured. |
| B5 | Interrupt | Stopping mid-turn halts output, and the pane accepts the next prompt. |
| B6 | Resume | After closing and reopening the pane, history restores and the next prompt continues the same conversation. |
| B7 | Model and effort | The model and effort chosen in the menu are the ones actually used, according to the CLI's own report or the request log. This covers #4161 and related bugs. |
| B8 | AgentMux MCP | The agent can call `WhoAmI` through the agentmux MCP server. |
| B9 | Inbound message | Another agent's message (jekt) sent to the agent while it's idle arrives and starts a turn. |

Providers that are unpublished or not installed (muxcode, antigravity, kimi) are checked as far as they go. "Not installable" is a valid, recorded outcome.

The results go in a `REPORT_PROVIDER_HARNESS_PASS_<date>.md` next to this spec. Each failure becomes an issue or a fix PR that links back to the report. For providers whose event streams are parsed by a translator (claude, codex, gemini, qwen), capture the B3 and B4 sessions as fixtures when that's cheap, so the next pin bump can replay them the way codex's fixtures already do.

## 7. Progress

| Phase | Item | PR | State |
|---|---|---|---|
| 1a | claude 2.1.287, gemini 0.62.0, qwen 0.24.7, openclaw 2026.9.7, copilot 1.0.91 | this PR | in review |
| 1b | codex 0.160.0 | — | not started |
| 2 | cef-dll-sys, Node, jq, ripgrep, rustls, toolchain | — | not started |
| 3 | ed25519-dalek, marked | — | not started |
| 4 | reqwest, rusqlite, keyring, katex, shiki, mermaid, vite, typescript | — | not started |
| 5 | provider harness pass | — | not started |

## 8. Non-goals

- Lambda runtime changes. They live in `shared-infrastructure`, and the report marks them `current`.
- The drift reporter itself. Its limitations are tracked in `shared-infrastructure`'s reporter specs.
- Model catalog changes, since everything there is `current`.
