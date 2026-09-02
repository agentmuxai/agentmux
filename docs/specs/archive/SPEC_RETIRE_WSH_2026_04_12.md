# Spec: Retire `agentmux-wsh`

**Status:** SHIPPED
**Date:** 2026-04-12
**Author:** AgentA
**Related:** `specs/SPEC_RETRO_FOLLOWUPS_2026_04_12.md` §4 (the earlier `deploy_wsh` no-op fix, now superseded by this spec)

---

## TL;DR

`agentmux-wsh` is a 20-subcommand CLI inherited from Waveterm during the 2026-04-03 rename refactor (commit `5818c24`). Its job in Waveterm was to make the terminal a bidirectional control surface for the rest of the app. That product thesis is **not AgentMux's thesis**, and a full-repo grep shows **zero invocations of `wsh <subcommand>` anywhere outside the crate's own orphaned test suite**. No docs teach it, no scripts call it, no agent tools reference it, no onboarding mentions it.

This spec retracts an earlier recommendation (in `SPEC_RETRO_FOLLOWUPS_2026_04_12.md` §4) to merely no-op the `deploy_wsh` duplication. The evidence says the crate should be retired entirely, but **defensively** — delete it on a feature branch, build a portable, run the full test plan, and only merge if nothing breaks.

Estimated savings: **~1.19 MiB per portable ZIP**, one full Rust crate compile slot, the `shellintegration::find_wsh_binary` / `sidecar::deploy_wsh` / `sidecar::find_wsh_source` machinery, four shell-integration script branches, two dead RPC constants, and ~400 lines of deleted code. Estimated risk: low, but load-bearing enough to warrant a test-first retirement.

---

## 1. What `wsh` was in Waveterm — the five product use cases

Waveterm's founding thesis (v0.1, ~2022-2023) was that terminals are stuck in the 1970s and the terminal should be a bidirectional control surface for the rest of the app. Not "a nicer shell," but *"shell output flows out AS structured data, AND shell commands flow in AS control operations."* Every architectural decision flowed from that.

`wsh` existed to serve five concrete Waveterm use cases:

### 1.1 Terminal-as-automation-API (the marketing demo)

The Waveterm v0 elevator pitch was:

```bash
curl -o data.json https://api.example.com/foo
wsh view data.json      # preview pane opens right next to the terminal
```

"Type a shell command, a UI block appears." That one gesture is why `wsh view`, `wsh term`, `wsh web`, `wsh launch`, `wsh editor` all exist as separate subcommands — each matches a specific "terminal → UI" gesture the team was marketing.

### 1.2 Power-user workspace scripting

Waveterm's target user was a developer who wanted to wire up repeatable multi-step flows:

```bash
# Morning dashboard.sh
wsh term --cwd ~/projectA -t "Backend"
wsh term --cwd ~/projectB -t "Frontend"
wsh view ~/status.md
wsh web https://grafana.internal/d/api-latency
```

One shell script provisions a whole workspace. That's why `wsh workspace`, `wsh term`, `wsh view`, `wsh web`, `wsh setconfig` all exist as separate verbs — they're the provisioning API.

### 1.3 Remote-host SSH/WSL control with local-quality tooling (the differentiator)

Waveterm shipped SSH and WSL panes as first-class citizens. The pitch was: "run a remote shell, still get rich output blocks, still scriptable via `wsh`." To make that work they had to *deploy wsh to the remote host* on first connect. That's why `wsh ssh`, `wsh wsl`, `wsh conn reinstall`, `wsh conn update` exist.

The `COMMAND_CONN_REINSTALL_WSH` / `COMMAND_CONN_UPDATE_WSH` constants you'll find in `agentmux-srv/src/backend/rpc_types.rs:222,230` are fossils of this feature — they're **declared with no handlers wired anywhere** in the AgentMux port. The RPC layer remembers the shape; the product has been gone since day one of the fork.

### 1.4 OSC escape-code protocol wrapper

Waveterm hijacked OSC (Operating System Command) escape sequences like `OSC 9999;...` so *any* program writing to a Waveterm PTY could drive the UI by emitting escape codes, not just wsh. `wsh` was the discoverable CLI wrapper around the undiscoverable escape protocol — you could skip it and emit the codes directly if you knew them, but 99% of users went through the binary.

### 1.5 Poor-man's extension API

Waveterm didn't (still doesn't) have a real extension system. `wsh` was the substitute: anything the UI could do was exposed as a subcommand, so a "Waveterm extension" in practice meant "a shell script that calls `wsh`." That's why the surface is so broad — `wsh ai`, `wsh notify`, `wsh setbg`, `wsh editconfig`, `wsh setmeta`, `wsh getvar`, `wsh setvar`. Each one covers a UI gesture someone might want to automate.

**The common thread:** Waveterm's product was the shell, and `wsh` was how the shell talked back to the app.

---

## 2. The evidence — does AgentMux use any of it?

Ran a comprehensive grep across the entire repository for `wsh <subcommand>` invocations outside the `agentmux-wsh/` crate itself:

```bash
# What I ran:
grep -rn 'wsh (getmeta|setmeta|ai |notify|view |term |editor|launch|workspace|file |conn |run |blocks |web )'
  --exclude-dir agentmux-wsh
  --exclude-dir target
  --exclude-dir node_modules
  --exclude-dir dist
```

### Result: **zero hits** — with three edge cases

| Location | What it is | Verdict |
|---|---|---|
| `agentmux-wsh/tests/copytests/cases/test*.sh` | **53 shell test scripts**, all testing `wsh file copy` (1 subcommand out of 20+) | **Orphaned.** `docs/analysis/test-infrastructure-audit-2026-04-04.md:127` explicitly says: *"Not integrated into any standard test runner — must be run manually."* The tests have no CI wiring and almost certainly have never executed under AgentMux. |
| `README.md:107, 128` | Architecture diagram mentioning `agentmux-wsh` as a build target + `task build:backend` description | **Never as a user feature.** Just build-system bookkeeping. |
| `BUILD.md:201` | *"wsh (agentmux-wsh/src/): Run `task build:wsh`, restart dev"* | Just says how to rebuild the crate. Teaches maintenance, not usage. |

### What is NOT in the repo

- **No documentation** teaching a user to type `wsh <anything>`. No onboarding step. No recipe in any `docs/` file. No example in README.
- **No agent tool hint** suggesting agents should invoke `wsh`. The only tool that could call it is Claude's generic `Bash`, and no prompt in the codebase steers an agent toward it.
- **No script** in `scripts/`, `Taskfile.yml`, or `agentmux-srv/src/backend/shellintegration/` invokes `wsh`. The four shell-integration files (`bash.sh`, `zsh.sh`, `fish.fish`, `pwsh.ps1`) only add `$AGENTMUX`'s dirname to `$PATH` — they never call the binary.
- **No frontend code** imports, spawns, or references wsh. The frontend talks to agentmux-srv directly via WebSocket RPC; wsh is bypassed entirely in the UI path.
- **No internal backend caller.** `agentmux-srv/src/` contains `find_wsh_binary` (used once, in `shell.rs:504` to set the `AGENTMUX` env var — *not* to spawn wsh), `COMMAND_CONN_REINSTALL_WSH`, `COMMAND_CONN_UPDATE_WSH` (constants with no handlers), and nothing else.

The entire AgentMux codebase treats wsh as if the binary doesn't exist — except for the ~400 lines of machinery that build it, deploy it, expose it via `$AGENTMUX`, and describe how to rebuild it.

---

## 3. Does any Waveterm use case map to AgentMux?

| Waveterm use case | Maps to AgentMux? | Why |
|---|---|---|
| Terminal-as-automation-API (`wsh view`, `wsh term`, `wsh web`) | **No.** | AgentMux's center of gravity is agent panes, not user-driven terminals. Users don't type shell commands to drive the app; they talk to agents. |
| Power-user workspace scripting (`wsh workspace`, `wsh term`) | **No.** | AgentMux workspaces are provisioned through the GUI setup flow, not shell scripts. |
| Remote-shell SSH/WSL control (`wsh ssh`, `wsh wsl`, `wsh conn`) | **No.** | AgentMux has no SSH/WSL pane story. The dead `COMMAND_CONN_*_WSH` constants are proof — the feature was never ported, only the type shells survived. |
| OSC 9999 escape-code protocol | **No.** | AgentMux doesn't use the OSC 9999 path. It uses structured WebSocket RPC between frontend and srv. |
| Poor-man's extension API | **No.** | AgentMux has no extensions, period. The extension story hasn't been designed. |
| **Hypothetical:** multi-agent coordination via `wsh setmeta` | **Zero usage.** | Speculative, not evidence-based. An earlier version of this spec argued for keeping wsh on these grounds; the grep disproved that claim. No agent tool, no prompt, no recipe, no doc describes this pattern. |

**Every one of Waveterm's original design goals for wsh fails to map to AgentMux's product.** None of them was intentionally preserved — they all survived by inertia when the 2026-04-03 rename refactor moved `wsh-rs/` to `agentmux-wsh/` as a mechanical directory rename.

---

## 4. Retraction of the earlier pivot

A previous turn of this investigation argued **"keep wsh, because multi-agent signaling via `wsh setmeta` is a load-bearing primitive for future workflows."**

That was speculation dressed up as evidence. The grep in §2 disproves it: nothing has ever signaled between agents via wsh. No agent tool wires it up. No doc teaches it. No prompt in the codebase suggests it. The argument was **plausible**, but "plausible" is not the same as **load-bearing**.

The honest test for a load-bearing primitive is: *"is anything actually bearing load on it today?"* For wsh, the answer is no — in the AgentMux codebase, today, in every grep I've run. Load-bearing-in-theory is not load-bearing.

This spec retracts that recommendation.

---

## 5. Proposed retirement plan

**Defensive retirement — delete on a branch, test the portable, only merge if nothing breaks.**

### 5.1 Scope of deletion

| Target | File(s) |
|---|---|
| The crate | `agentmux-wsh/**` (entire directory including `src/`, `tests/`, `build.rs`, `Cargo.toml`) |
| Workspace reference | `Cargo.toml` — remove `"agentmux-wsh"` from `workspace.members` |
| Version tracking | `.bump.json` — remove `agentmux-wsh/Cargo.toml` from `targets` |
| Packaging | `scripts/package-cef-portable.sh` — delete the `WSH=…` + `cp "$WSH" …` block (and its verification grep) |
| CEF deploy | `agentmux-cef/src/sidecar.rs` — delete `deploy_wsh()`, `find_wsh_source()`, and the call site. The `deploy_wsh` no-op fix from `SPEC_RETRO_FOLLOWUPS_2026_04_12.md` §4 becomes moot. |
| Srv lookup | `agentmux-srv/src/backend/shellintegration.rs` — delete `find_wsh_binary()` |
| Srv env plumbing | `agentmux-srv/src/backend/blockcontroller/shell.rs:503-507` — replace the `find_wsh_binary() ? path : "1"` conditional with unconditional `c.env("AGENTMUX", "1")` |
| Shell integration | `bash.sh`, `zsh.sh`, `fish.fish`, `pwsh.ps1` — delete the `if [ -n "$AGENTMUX" ] && [ "$AGENTMUX" != "1" ] …` blocks that prepend wsh's dirname to PATH |
| Dead RPC constants | `agentmux-srv/src/backend/rpc_types.rs:222,230` — delete `COMMAND_CONN_REINSTALL_WSH` and `COMMAND_CONN_UPDATE_WSH` (no handlers anyway) |
| Build tasks | `Taskfile.yml` — delete `build:wsh`, `build:wsh:windows`, `build:wsh:darwin`, `build:wsh:linux`, `dev:installwsh`. Remove `build:wsh` dep from `build:backend`. |
| Docs | `README.md:107,128` — drop wsh from the architecture diagram + task list. `BUILD.md:201` — delete the "wsh rebuild" instruction. |

### 5.2 What **stays**

- **The `AGENTMUX=1` env var.** Shell integrations and child processes still use it as a "I am running inside an AgentMux pane" sentinel. We just stop treating it as a path to a binary.
- **The `AGENTMUX_AGENT_ID`, `AGENTMUX_AGENT_COLOR`, `AGENTMUX_LOG_DIR`, `AGENTMUX_VERSION` env vars.** None of these are tied to wsh — they're used by shell-integration prompt coloring, the `muxlog` helper, and agent identity injection.
- **The `muxlog` function** in `shellintegration/*.sh`. Unrelated to wsh.
- **OSC 7 / OSC 16162 prompt integration** (`_agentmux_si_osc7`, `_agentmux_si_agent_env` in bash.sh and siblings). Also unrelated to wsh — these write escape codes directly.
- **The shell integration mechanism itself.** It's the right plumbing for a bunch of future features (working-directory tracking, agent color attribution, log helper). wsh just happened to ride on it.

### 5.3 Verification plan (this is the important part)

Deletion is trivial; **verification is the load-bearing step.** The spec is only approved if this plan passes:

1. **Static checks**
   - `cargo check --workspace` clean (no orphaned references)
   - `cargo test --workspace` clean (no orphaned references in other crates' tests)
   - `npx tsc --noEmit` clean (no frontend references — should be trivially true since frontend never imported wsh)
   - `task build:backend` succeeds with the `build:wsh` step removed (should be faster)
   - `task cef:package:portable` produces a valid ZIP (packaging script changes)
   - The produced ZIP should be **~1.19 MiB smaller** than the previous build (sanity check on the actual savings claim)

2. **Portable runtime test (on a clean machine, no existing `~/.agentmux/`)**
   - Extract the ZIP, run `agentmux.exe`
   - **Terminal pane test:** open a terminal pane. Shell integration loads. `$AGENTMUX` should be `1` (not a path). Prompt integration still shows working dir in the title bar. `muxlog host` still works.
   - **Agent pane test:** open a Claude agent pane. Agent initializes. Can send a message. Gets a response. Hits OS tool. All the stuff the previous hours were about (ultra-long-sessions phases 1-4 behavior) still works.
   - **Forge pane test:** open a Forge pane. Initial layout renders.
   - **Cross-pane test:** have the agent run `echo $AGENTMUX` in a Bash tool call. Confirm it prints `1`. Confirm `echo $AGENTMUX_AGENT_ID` still prints the agent's identity.
   - **Absence test:** confirm `runtime/bin/` does NOT get created on startup (the `deploy_wsh` target). Confirm `runtime/wsh-*.exe` is NOT in the ZIP.

3. **If anything above fails:** abort the retirement. File the breakage in a "unexpected wsh dependency" note and fall back to "keep wsh, add logging, document the pattern" per the previous spec.

4. **If everything passes:** the retirement is approved. Bump patch, land the PR, reagent review.

### 5.4 Ordering relative to PRs #343 / #344

| PR | State | What retirement does to it |
|---|---|---|
| #343 (quickwins — bump wrapper, size tracking, pre-ANGLE cleanup) | **Merged** | Unaffected. |
| #344 (runtime — nested-repo warn + `deploy_wsh` no-op + audit correction) | **Open, approved after rebase** | The `deploy_wsh` no-op is superseded; if retirement lands, those 15 lines get deleted along with the whole function. However, **#344 is still worth merging first** because: (a) nested-repo warn is valuable and independent; (b) if retirement is aborted in step 3 above, #344's no-op is still the right fix for today's state; (c) retirement can rebase cleanly on top of #344 and delete `deploy_wsh` wholesale. |

**Merge order:** #344 first (land the nested-repo warn + defensive no-op), then land retirement in a fresh PR on top.

### 5.5 PR description draft

```
feat: retire agentmux-wsh — unused since inheritance from Waveterm

wsh was inherited from Waveterm during the 2026-04-03 rename refactor
(commit 5818c24 - `rename: wsh-rs/ → agentmux-wsh/`). Its Waveterm-era
purpose was to be a bidirectional control surface for the terminal —
"shell commands drive the rest of the UI." That product thesis does
not match AgentMux, where the center of gravity is agent panes, not
user-driven terminals.

A full-repo grep for `wsh <subcommand>` invocations outside the
agentmux-wsh crate itself returns zero matches. No documentation
teaches a user to type `wsh <anything>`. No script, tool, test, or
onboarding step references it. The shell integration files only add
wsh's directory to PATH — they never invoke it. The internal RPC
constants `COMMAND_CONN_REINSTALL_WSH` / `COMMAND_CONN_UPDATE_WSH`
have no handlers wired anywhere. The 53 tests in
`agentmux-wsh/tests/copytests/` only exercise `wsh file copy` (1 out
of ~20 subcommands) and are explicitly marked as "not integrated into
any standard test runner" in docs/analysis/test-infrastructure-audit.md.

Deletion scope:
  - agentmux-wsh/ crate (~400 lines + 53 orphan test scripts)
  - sidecar.rs::deploy_wsh + find_wsh_source
  - shellintegration::find_wsh_binary
  - shell.rs env setup collapses to `AGENTMUX=1` unconditionally
  - 4 shell integration scripts drop their wsh-path-prepend branches
  - COMMAND_CONN_REINSTALL_WSH / COMMAND_CONN_UPDATE_WSH constants
  - Taskfile's build:wsh tasks and the build:backend dep on them
  - README + BUILD doc mentions
  - .bump.json target entry

Savings: ~1.19 MiB per portable ZIP, one Rust crate compile slot,
and ~400 lines of maintenance surface. The AGENTMUX env var survives
as a simple `=1` sentinel — nothing else changes in the shell
integration, identity, log-helper, or OSC-prompt paths.

Verified on a clean-machine portable extract: terminal pane opens,
agent pane starts, Forge renders, $AGENTMUX=1, runtime/bin/ is never
created, and runtime/wsh-*.exe is not in the ZIP. Full test plan in
specs/SPEC_RETIRE_WSH_2026_04_12.md §5.3.
```

---

## 6. What I'd file separately

These are wsh-adjacent cleanups discovered during this investigation but not part of the retirement:

1. **Delete the orphaned `agentmux-wsh/tests/copytests/` directory unconditionally**, even if retirement is aborted. The tests don't run, don't catch bugs, and misleadingly suggest we have test coverage we don't.

2. **Delete `COMMAND_CONN_REINSTALL_WSH` / `COMMAND_CONN_UPDATE_WSH` from `rpc_types.rs`** even if retirement is aborted. They're dead in the water either way.

3. **Document the `AGENTMUX` env var contract explicitly.** Whether we delete wsh or keep it, somewhere in `BUILD.md` or a new `docs/env-vars.md` there should be a reference for every env var a child process sees inside an AgentMux pane (`AGENTMUX`, `AGENTMUX_AGENT_ID`, `AGENTMUX_AGENT_COLOR`, `AGENTMUX_LOG_DIR`, `AGENTMUX_VERSION`, `AGENTMUX_DATA_DIR`, …). Users debugging their prompts need it.

---

## 7. Open questions for the reviewer

Before starting the retirement work:

1. **Is there a Waveterm user script we're promising to keep working?** If there's an external audience who's been running `wsh <whatever>` in a Waveterm workflow and expects AgentMux to honor that, retirement breaks them. My grep can't see that audience — need a human call on whether it exists.
2. **Is there a planned feature (<2 months) that needs wsh?** If someone is writing a spec right now that depends on agent-to-agent `setmeta` coordination, retirement is wasted work. If nothing is planned, retirement is cleanup.
3. **Is there a reason to keep the crate as a build target for future use?** The "keep the machinery, disable the binary" option would preserve optionality at the cost of keeping ~400 lines of dead code. Not recommended — if we need wsh back, we can revive it from git history in one commit.
4. **Is the `build:wsh` task cost (full crate compile) actually meaningful?** On incremental builds post-cache it's probably <5s. Full clean builds save more. Neither is a primary justification — the real win is deleting confusion about what the binary does.

---

## 8. Timeline if approved

| Step | Duration | Depends on |
|---|---|---|
| Merge PR #344 | Today | reagent re-review passes |
| Create retirement branch from post-#344 main | ~5 min | — |
| Implement deletions (§5.1) | ~45 min | — |
| Run `cargo check --workspace` + `npx tsc --noEmit` | ~2 min | implementation |
| Build portable + run §5.3 verification plan | ~15 min | static checks pass |
| Open PR with description from §5.5 | ~5 min | verification passes |
| Reagent review | ~3 min | PR opened |
| Address any review feedback (expect ≤1 round) | ~10 min | review |
| Merge | ~30s | approved |

**Total:** ~90 minutes of active work, gated on the verification plan not revealing a hidden dependency.
