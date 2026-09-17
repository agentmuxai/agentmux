# SPEC: an instance's identity must not be inheritable by its panes

**Author:** Opaz
**Date:** 2026-09-17
**Status:** implemented

---

## 1. Problem

Both pane-spawn paths build their environment by inheriting srv's, and srv's
environment is exactly where instance identity lives:

| Pane type | Spawner | Base environment |
|---|---|---|
| Terminal | `portable_pty::CommandBuilder` (`shell/lifecycle.rs`) | seeded from `std::env::vars_os()`; `.env()` calls are overrides on a full copy |
| Agent | `tokio::process::Command` (`blockcontroller/core.rs::apply_working_dir`) | std default: full inheritance |

srv receives the whole identity set from the launcher
(`DataPaths::to_env_vars` — data dir, config dir, cache dir, channel, runtime
mode) and adds more itself (`AGENTMUX_DATA_HOME`, `AGENTMUX_CONFIG_HOME`,
`AGENTMUX_LOCAL_URL`).

So every `AGENTMUX_*` variable reached every pane and every process launched
from one — **including another AgentMux build, which then adopted the launching
instance's identity instead of the one baked into it at compile time.**

`env_clear` appeared zero times in the repo. The entire pane-side sanitization
surface was five `env_remove` calls for agent-identity vars in one branch.

Confirmed failures, all observed live (full RCA:
`docs/retro/retro-env-inheritance-instance-isolation-breach-2026-09-17.md`):

1. A 0.56.3 build launched from a pane inherited `AGENTMUX_CHANNEL` and wrote
   into a 0.56.2 instance's channel.
2. `AGENTMUX_DATA_HOME` inherited by a dev srv made `get_mux_data_dir()` resolve
   another instance's data dir before bootstrap corrected it.
3. `AGENTMUX_EXTRACTED_RUN=1` inherited by a freshly built AppImage made it skip
   extraction and run the previous build's binary (fixed separately, #3324).

This is invariant **I6** breached through a channel I1–I6 never named: they
enumerate named OS objects and directories, and their threat model is "a new
launch must not crash a running one". This is neither — it is silent, and it
corrupts the *new* instance from the *old* one.

## 2. Why the existing guard did not cover it

`agentmux-launcher/src/data_dir.rs` already ignores ambient `AGENTMUX_*` and
re-derives from its own exe path when the bare `AGENTMUX` sentinel is present.
That is the right mitigation and it predates this spec.

It was set in exactly one live place — the interactive shell-pane branch
(`shell/lifecycle.rs`). The only other setter, `shellexec.rs::build_mux_env`,
has no callers. **The agent-pane path never set it**, so a build launched from
an agent's shell tool had `nested == false` while carrying the launching
instance's channel, and the leak was honoured as a deliberate override.
`docs/retro/retro-portable-0496-pool-window-false-close-2026-06-27.md:143-145`
predicted this exact hole.

## 3. Design

### 3.1 Allowlist, not denylist

`agentmux-srv/src/backend/pane_env.rs` owns the policy. `should_strip(key)` is
true for any key starting with `AGENTMUX` that is not the sentinel and not in
`PANE_ENV_KEEP`.

The direction matters. A denylist leaves every future variable leaking until
someone remembers to deny it; the previous state of the world was an implicit
allow-everything, and this inverts it. `keys_to_strip()` reads the **live**
environment rather than a hardcoded list, so a variable introduced anywhere —
including by a dependency or a future `set_var` — is stripped without anyone
editing this file.

### 3.2 The keep-set

Derived from an exhaustive audit of every `process.env` read under
`backend/shellintegration/` plus the shell rc scripts. Each entry has a stated
reason in the source; anything without one does not belong.

- `AGENTMUX_LOCAL_URL`, `AGENTMUX_AUTH_KEY` — `lib/muxclient.mjs` reads exactly
  these two and every helper CLI goes through it.
- `AGENTMUX_BLOCKID`, `AGENTMUX_TABID` — which pane the helper speaks for.
- `AGENTMUX_VERSION`, `AGENTMUX_LOG_DIR` — prompt hook and `muxlog` banner.
- `AGENTMUX_AGENT_ID`, `_COLOR`, `_TEXT_COLOR`, `_SLUG`, `_DISPLAY`,
  `AGENTMUX_INSTANCE_SLUG` — OSC-16162 prompt hook, `gh-agent.sh` credential
  selection, bashwrap's per-instance cwd-state key.

Eleven high-risk variables have **zero** in-pane consumers and are stripped with
no loss at all: `AGENTMUX_INSTANCE_DIR`, `AGENTMUX_CEF_CACHE_DIR`,
`AGENTMUX_AGENTS_DIR`, `AGENTMUX_INSTANCE_RUNTIME_DIR`, `AGENTMUX_DATA_HOME`,
`AGENTMUX_CONFIG_HOME`, `AGENTMUX_APP_PATH`, `AGENTMUX_SRV_PIPE_PATH`,
`AGENTMUX_PATH_SOURCE`, `AGENTMUX_EXTRACTED_RUN`, `AGENTMUX_SPLASH_READY_FILE`.

### 3.2a Every spawn path, via one helper

The first revision applied the strip inline at a single call site and missed
most of them. Review (ReAgent, two P0s on PR #3326) found that
`shell/lifecycle.rs` builds its `CommandBuilder` in **three** branches — direct
spawn (agent CLIs launched with args), shell-wrapped `cmd`, and the interactive
shell — and only the third was patched, so a directly-launched agent CLI still
inherited everything. It also found `shell_node.rs` (`POST /api/v1/shell/create`,
the MCP `Shell` tool that CLAUDE.md names as the way for an agent to launch
`task dev`/`task package`) entirely unpatched — the single most likely path for
this bug to occur in practice.

A follow-up sweep of every spawn construction in `agentmux-srv` found a third
agent-facing path the review had not named: `server/shell_handlers.rs`'s
`shellexec`, whose own comment notes it "fires on every MCP Shell tool call".

So the policy is now applied through one helper per command type —
`pane_env::sanitize_pty_command` / `sanitize_process_command`, each doing strip
*and* sentinel — at four sites:

| Site | What it spawns |
|---|---|
| `shell/lifecycle.rs` (after the if/else) | all three PTY branches |
| `blockcontroller/core.rs` | agent subprocesses |
| `backend/shell_node.rs` | `/api/v1/shell/create` — MCP `Shell` |
| `server/shell_handlers.rs` | `shellexec` — MCP `Shell` calls |
| `server/identity_auth_spawn.rs` ×2 | provider CLIs for OAuth login |

### 3.2b A stricter policy for processes that are not ours

Review round 2 found both spawn sites in `identity_auth_spawn.rs` unpatched: the
PTY and non-PTY paths that launch a provider CLI for OAuth login. Those are
third-party, network-connected binaries, and they were inheriting the full
instance identity.

They get `sanitize_external_command` / `sanitize_external_pty_command`, which
strip **every** `AGENTMUX_*` including the helper keep-set. `muxsh` needs
`AGENTMUX_LOCAL_URL` and `AGENTMUX_AUTH_KEY`; `claude login` does not, and
handing an external binary this instance's API endpoint — or the credential to
it — is the one case where the keep-set is plainly too generous. The nesting
sentinel still goes through: it carries no identity.

This was in my own sweep output and I deprioritised it as "less about launching
builds". That was the wrong call — it is the most security-relevant site in the
list.

Applying it *after* the if/else rather than inside a branch is deliberate: a
per-branch call is exactly the shape that silently half-applies, which is how
the first revision shipped a fix that left the most important path open.

### 3.2c The complete spawn inventory

Three review rounds each found sites the previous fix missed, every time because
the fix was applied where the author happened to be looking rather than to an
enumerated list. So: every `Command::new` / `CommandBuilder::new` in
`agentmux-srv`, classified. Grep for the constructor, not for one alias — round 3
was missed because `agents/runner.rs` imports `Command` rather than writing
`tokio::process::Command`.

**Pane policy** (`sanitize_pty_command` / `sanitize_process_command`) — ours, may
use the in-pane helpers, so [`PANE_ENV_KEEP`] applies:

| Site | Spawns |
|---|---|
| `blockcontroller/shell/lifecycle.rs` | all three PTY branches |
| `blockcontroller/core.rs` | agent subprocesses |
| `agents/runner.rs` | the drone one-shot agent CLI |
| `backend/shell_node.rs` | `/api/v1/shell/create` — MCP `Shell` |
| `server/shell_handlers.rs` | `shellexec` |

**Strict policy** (`sanitize_external_command` / `..._pty_command`) — third-party,
so not even the keep-set:

| Site | Spawns |
|---|---|
| `server/identity_auth_spawn.rs` ×2 | provider CLIs for OAuth login |
| `backend/lsp/supervisor.rs` | language servers |
| `backend/mcp_probe.rs` | an arbitrary configured MCP command |
| `server/voice.rs` | whisper.cpp |
| `server/install_handlers.rs` | `npm install` — arbitrary postinstall scripts |
| `server/system_install_handlers.rs` | an arbitrary install-recipe program |

**Deliberately not sanitized**, with reasons:

- `crash_monitor.rs` re-spawns **our own executable** to recover. Stripping would
  make the replacement lose the identity it is supposed to resume.
- Short-lived probes — `which`/`where`, `rustc --version`, `apt-cache`,
  `npm --version`, `brew`, `winget` queries. They read and exit; there is nothing
  to leak into and patching them is churn that dilutes review.
- `util.rs`'s `open`/`xdg-open`/`explorer.exe` hand a URL or path to the desktop
  handler. Arguably external, but stripping risks breaking the handoff on some
  desktops for no concrete gain; noted rather than changed.

### 3.3 Strip before the overlay

Both call sites strip *before* applying their own explicit `.env()` values. The
terminal path re-sets everything a pane needs a few lines later, so stripping
first costs nothing there and keeps the allowlist honest rather than
load-bearing.

### 3.4 Close the sentinel gap

`apply_working_dir` now sets `AGENTMUX` on the agent path, so the launcher's
existing nested-guard finally fires for the path every confirmed failure came
through.

## 4. Deliberately NOT done

**`AGENTMUX_LOCAL_URL` + `AGENTMUX_AUTH_KEY` remain inheritable**, and they are
the largest residual coupling: a process inheriting both holds an authenticated
handle to the instance that spawned it, and `AGENTMUX_AUTH_KEY` is the sole auth
factor for the whole App API (`server/mod.rs`, a string equality against a
per-launch UUID with no scoping, expiry or rate limit).

Removing them is a redesign of how helpers reach the server — a per-invocation
handoff over a well-known per-instance socket — not a strip, and it would break
`muxsh`/`muxlog`/`muxspect`/`muxopen` on the way. Tracked as follow-up in the
retro §9.4. Calling this spec "strict isolation" without naming that exception
would be false.

Also not done here: making the sentinel self-describing (tmux's
`TMUX=socket,pid,session` shape). Presence-only is the *safe* direction — an
inherited sentinel means "ignore ambient vars", which is what we want — so this
is an improvement, not a correctness gap.

## 5. Tests

`backend::pane_env` unit tests:

- `the_identity_vars_that_caused_the_breach_are_stripped` — pins the four
  variables behind the confirmed failures plus the rest of the identity set, so
  the breach cannot be reopened by quietly adding one to the keep-set.
- `an_unknown_agentmux_var_is_stripped_by_default` — the allowlist property;
  this is what makes the fix survive variable #21.
- `the_helper_keep_set_survives`, `non_agentmux_vars_are_left_alone`,
  `the_nesting_sentinel_is_never_stripped`, `keys_to_strip_reads_the_live_environment`.

Verified live: a terminal pane in a dev instance carrying the fix shows only the
keep-set, with the identity variables absent.

## 6. Follow-up

1. Replace the ambient credential with a per-invocation handoff (§4).
2. Add an **I7** invariant — "an instance's identity must not be inheritable by
   processes it spawns" — with an enforcement test that launches a build from a
   pane and asserts it resolves its own channel and data dir. The absence of
   such a test is why five prior documents identified this class and none closed
   it.
3. Reconcile three specs that contradict each other about whether
   `AGENTMUX_AUTH_KEY` is present in panes (code: absent from shell panes,
   present in agent panes).
