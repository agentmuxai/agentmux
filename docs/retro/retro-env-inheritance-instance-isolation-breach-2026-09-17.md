# Retro / RCA: environment inheritance is an unguarded instance-isolation channel

**Date:** 2026-09-17
**Author:** Opaz
**Status:** active — root cause confirmed. Shipped: sender-side sanitization
(`SPEC_PANE_ENV_ISOLATION_2026_09_17`); the §9.5 **I7 invariant + enforcement
test** (#3365), which also closed two spawn paths that were violating it live
(`open_browser`, reveal-in-file-manager). Remaining: the §9.4
ambient-credential redesign, designed in `SPEC_PANE_CREDENTIAL_HANDOFF_2026_09_18`
(#3358) but not implemented; and the end-to-end check that launches a build
from a pane and asserts it resolves its own channel and data dir — #3365
enforces spawn-site coverage in source, not runtime behaviour.
**Severity:** High — silent cross-instance data-dir/channel coupling, plus a live
App API credential reaching every agent pane and its descendants
**Scope:** all platforms; both pane types; all versions carrying `DataPaths::to_env_vars`

---

## 1. Summary

A running AgentMux instance publishes its own identity — data dir, config dir,
cache dir, channel, runtime mode, IPC endpoint, and an API credential — into the
environment of the shells inside its panes. **Neither pane-spawn path clears or
filters the environment**, so every one of those variables is inherited by every
process started from a pane, including another AgentMux build.

The consequence is that a second instance launched from inside a first one does
not get the identity it was built with. It silently adopts the launching
instance's. Per-build channel isolation, which the packaging pipeline goes to
real trouble to bake in at compile time, is overridden at runtime by an
inherited string.

`env_clear` appears **zero times** in this repository. The entire pane-side
sanitization surface is five `env_remove` calls for agent-identity variables in
one branch of one function (`shell/lifecycle.rs:717-725`).

## 2. Impact — four confirmed failures

All four were observed live on 2026-09-16/17, not inferred.

### 2.1 A freshly built AppImage ran the wrong binary
`AGENTMUX_EXTRACTED_RUN=1` is exported by `scripts/linux-apprun.sh` before it
re-execs from its extraction cache. It is read by nothing else. Inherited into
panes, it made a newly built AppImage take AppRun's "already extracted" branch
and run from its FUSE mount — no extraction, no cache check, no message. The
build under test was never the build that ran.

### 2.2 A new build joined the launching instance's channel
`AGENTMUX_CHANNEL` overrides the compile-time `AGENTMUX_BUILD_CHANNEL_DEFAULT`
(`agentmux-common/src/data_paths.rs:54-58`, `option_env!`). A build launched from
a pane inherited `verify-decrqm-fix` and wrote `channels/verify-decrqm-fix/versions/0.56.3/`
(20 MB) into the launching instance's channel instead of its own baked one.

### 2.3 A dev srv resolved another instance's data dir
`AGENTMUX_DATA_HOME` is read by `backend/base.rs:69` (`get_mux_data_dir`).
`bootstrap::open_stores_and_migrate` overwrites it from resolved config — but
until that runs, an inherited value stands. Observed: a `task dev` srv launched
from a terminal inside a 0.56.2 instance carried that instance's data dir in its
environment. Callers of `get_mux_data_dir()` that run before bootstrap therefore
resolve the wrong instance.

### 2.4 A live API credential reaches every agent pane
`AGENTMUX_AUTH_KEY` in a pane shell is **byte-identical** to the instance's
`authkey.dev` `auth_key` — verified directly. It is the sole authentication
factor for the whole App API: `server/mod.rs:2711-2753` is a string equality
against a per-launch UUIDv4, with no scoping, expiry, per-caller identity or rate
limit, and `/ws` additionally accepts it as a `?authkey=` query parameter.
`AGENTMUX_LOCAL_URL` supplies the matching endpoint.

So any process started from an agent pane — an npm postinstall, a downloaded
binary, a second AgentMux build — inherits a working `(endpoint, credential)`
pair for the *launching* instance, and can do anything the UI can: create and
kill panes and shells, open editors, read and write block files, drive the UI,
send messages as any agent, manage cron, read native and global memory. On Linux
the value is also readable by any same-uid process via `/proc/PID/environ`.

The in-instance version of this is a known and accepted trust boundary —
`service/credential.rs:315-321` names it explicitly. **The cross-instance version
is not documented anywhere.**

Note the asymmetry, which had misled earlier reasoning: `config.rs:98` removes
`AGENTMUX_AUTH_KEY` from srv's own environment, so it does **not** reach
interactive shell panes — but it is deliberately re-injected per agent spawn at
`agent_handlers/input.rs:406`, so it **does** reach agent panes and everything
they launch. Three specs disagree about this; see §7.

## 3. Root cause

Two spawn paths build a pane's environment, and neither filters:

| Pane type | Spawner | Base environment |
|---|---|---|
| Terminal | `portable_pty::CommandBuilder` (`shell/lifecycle.rs:559`) | seeded from `std::env::vars_os()` (`portable-pty-0.9.0/src/cmdbuilder.rs:75`); `.env()` calls are overrides layered on a **full copy of srv's environment** |
| Agent | `tokio::process::Command` (`subprocess/host_spawn.rs:155` → `core.rs:77-80`) | std default: full inheritance; `env_vars` overlaid |

srv's own environment is loaded by the launcher at `srv_spawner.rs:447`
(`.envs(paths.common.to_env_vars())`), which injects the entire instance identity
set from `data_paths.rs:268-295`, and srv then adds more via `set_var`
(`bootstrap.rs:408, 487, 490, 493, 1507`).

So the leak is not an oversight in one call site. It is the default behaviour of
both spawn paths, applied to a deliberately-constructed identity set.

## 4. Why the existing guard did not fire

A guard already exists. `agentmux-launcher/src/data_dir.rs:60-99`:

```rust
let nested = std::env::var_os("AGENTMUX").is_some();   // :77
let ignore_ambient = is_dev || nested;
```

When the bare `AGENTMUX` sentinel is present, the launcher ignores ambient
`AGENTMUX_*` and re-derives from its own exe path. That is exactly the right
mitigation.

**It is set in only one live place: `shell/lifecycle.rs:600`, the interactive
shell-pane branch.** The only other setter, `shellexec.rs:351`, sits inside
`build_mux_env`, which has **no callers**.

The agent-pane path never sets it. A build launched from an agent's shell tool
therefore has `nested == false` while still carrying the launching instance's
`AGENTMUX_CHANNEL` and `AGENTMUX_RUNTIME_MODE`, so the leaked channel is honoured
as a deliberate standalone override.

The observed environment confirms which path we were on: it contains
`AGENTMUX_AGENT_ID`, `AGENTMUX_AUTH_KEY` and `AGENTMUX_BLOCKID` (agent-spawn
injections) but **no `AGENTMUX`, no `AGENTMUX_TABID`, no `AGENTMUX_VERSION`** —
an agent pane, not a terminal pane. Every failure in §2 was reproduced through
the one path the guard does not cover.

## 5. Why we missed it — and the uncomfortable part

**The isolation invariants do not mention the environment.** I1-I6 (`CLAUDE.md`)
enumerate named OS objects and directories — pipes, job objects, PIDs, data dirs
— and their stated threat model is "launching a new build must never *crash* a
running one". Environment inheritance is neither a named OS object nor a crash,
and it runs the opposite way: the *new* instance is silently corrupted by the
*old* one. The I6 enforcement-test list contains no test for hostile or leaked
ambient environment.

**But this was not unknown.** There is a five-document lineage:

| Document | What it already said |
|---|---|
| `docs/specs/dev-build-env-isolation.md` (2026-05-06) | The canonical original: `task dev` from a pane inherited `AGENTMUX_DATA_DIR` "and the rest of the `AGENTMUX_*` family"; resolution was "dev builds never inherit `AGENTMUX_*` env vars" |
| `docs/specs/SPEC_DEV_ENV_ISOLATION.md` (2026-06-24) §2.4 | Documents the `AGENTMUX=1` sentinel guard verbatim |
| `docs/retro/retro-per-build-launch-isolation-2026-06-13.md:54` | "Env leaks cross the process boundary. `AGENTMUX_CHANNEL` set for a pane shell silently redirects anything launched from it." |
| `docs/specs/SPEC_DEV_BADGE_RUNTIME_MODE_2026_06_12.md:226` | Open question 3, **never answered**: "Scrub-list scope — exactly which `AGENTMUX_*` vars are 'instance identity' (must not inherit) vs legitimately inheritable?" |
| `docs/retro/retro-portable-0496-pool-window-false-close-2026-06-27.md:143-145` | Predicted this precise hole: the guard "only fires when `AGENTMUX=1` is set… a portable launched from a terminal that inherited `AGENTMUX_RUNTIME_MODE=dev:main` but NOT `AGENTMUX=1` would mis-classify" |

So the honest finding is not "we never considered it". It is: **the problem was
identified five times, mitigated once (partially), the scoping question was asked
explicitly and left open, and the predicted residual hole is the one that bit
us.** Each fix was receiver-side — every launcher re-deriving from its own exe
path — and no one ever closed the sender side.

Two genuinely new things: `AGENTMUX_EXTRACTED_RUN` appears in no document in the
repo, and the cross-instance credential exposure is undocumented.

## 6. Inventory

Full table in the audit appendix; the shape is what matters:

- **High risk, reaches both pane types** — `AGENTMUX_CHANNEL`, `AGENTMUX_DATA_DIR`,
  `AGENTMUX_CONFIG_DIR`, `AGENTMUX_CEF_CACHE_DIR`, `AGENTMUX_INSTANCE_DIR`,
  `AGENTMUX_AGENTS_DIR`, `AGENTMUX_INSTANCE_RUNTIME_DIR`, `AGENTMUX_RUNTIME_MODE`,
  `AGENTMUX_DATA_HOME`, `AGENTMUX_CONFIG_HOME`, `AGENTMUX_LOCAL_URL`,
  `AGENTMUX_EXTRACTED_RUN`
- **High risk, agent panes only** — `AGENTMUX_AUTH_KEY`
- **Medium** — `AGENTMUX_APP_PATH`, `AGENTMUX_LOG_DIR`, `AGENTMUX_SRV_PIPE_PATH`,
  `AGENTMUX_SPLASH_READY_FILE`, `AGENTMUX_VERSION`, `AGENTMUX_CLONE_ID`,
  agent-identity vars
- **Correctly contained** — `AGENTMUX_HOST_REG_SECRET` (scrubbed at `config.rs:141`),
  per-agent signing keys (`AGENTMUX_JEKT_KEY`, `_LAN_KEY`, `_WAN_KEY` — MCP process
  only, never a pane)

`AGENTMUX_HOST_REG_SECRET` is the existing precedent that a scrub is both possible
and already considered correct for the most sensitive value.

**What the in-pane helpers actually need** (exhaustive, every `process.env` read
under `backend/shellintegration/`): `AGENTMUX_LOCAL_URL`, `AGENTMUX_AUTH_KEY`,
`AGENTMUX_BLOCKID`, `AGENTMUX_TABID`, `AGENTMUX_VERSION`, `AGENTMUX_LOG_DIR`,
`AGENTMUX_AGENT_ID`, `AGENTMUX_AGENT_COLOR`.

**Eleven high-risk vars have zero in-pane consumers and are strippable with no
loss at all**: `AGENTMUX_INSTANCE_DIR`, `AGENTMUX_CEF_CACHE_DIR`,
`AGENTMUX_AGENTS_DIR`, `AGENTMUX_INSTANCE_RUNTIME_DIR`, `AGENTMUX_DATA_HOME`,
`AGENTMUX_CONFIG_HOME`, `AGENTMUX_APP_PATH`, `AGENTMUX_SRV_PIPE_PATH`,
`AGENTMUX_PATH_SOURCE`, `AGENTMUX_EXTRACTED_RUN`, `AGENTMUX_SPLASH_READY_FILE`.

Stripping `AGENTMUX_DATA_DIR`/`CONFIG_DIR`/`SHARED_DIR` breaks only `muxsh
config-path` and `muxsh config-edit`, both of which already fail loudly and could
use an API round-trip. Stripping `AGENTMUX_CHANNEL`/`RUNTIME_MODE` costs `muxlog`
a ranking *hint* and `muxspect` its tier-0 heuristic, which its own code
documents as unreliable.

## 7. Documentation contradiction to reconcile

`SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md:36` asserts `AGENTMUX_AUTH_KEY` is
"*not* a PTY env var for trust reasons". `docs/MUXSH.md:40`,
`docs/agent-identity-bootstrap.md:49` and
`SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md:169-178` all state it is
present in panes. The code resolves it: absent from shell panes, present in agent
panes. One spec is stale and should be corrected rather than left to mislead the
next investigation.

## 8. Industry practice

- **VS Code** solves exactly this with sender-side sanitization:
  `sanitizeProcessEnvironment()` strips `^ELECTRON_.+$`,
  `^VSCODE_(?!(PORTABLE|SHELL_LOGIN|ENV_REPLACE|ENV_APPEND|ENV_PREPEND)).+$`,
  `^SNAP(|_.*)$` with an explicit preserve list, plus `removeDangerousEnvVariables()`
  for `DEBUG`, `NODE_OPTIONS`, `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`. Note the
  shape: a narrow allowlist carved out of a prefix-wide denylist.
  Their failure modes match ours one-for-one — `ELECTRON_NO_ASAR` leaking and
  breaking child Electron apps (#38428) is our §2.1; `VSCODE_IPC_HOOK_CLI` going
  stale in tmux (#157275) is our `AGENTMUX_LOCAL_URL`.
- **tmux and screen** make the sentinel *self-describing* rather than boolean:
  `TMUX=socket_path,server_pid,session_id`, `STY=pid.tty`. An inherited copy can be
  checked against a live server, so nesting is detected rather than silently
  honoured. Our bare `AGENTMUX=1` is the weaker form, and `AGENTMUX_EXTRACTED_RUN=1`
  was the weakest — a flag with no referent at all.
- **AppImage/Snap/Flatpak** hit this constantly (leaked `LD_LIBRARY_PATH`,
  `PYTHONHOME`, `APPDIR` breaking system tools); the established pattern is saving
  `ORIGINAL_*` values and restoring them for children. Ghostty 1.3.1 shipped
  precisely this fix. Flatpak passes the environment through unless `--clear-env`.
- **Secrets in environment**: the general debate is contested, but our case is the
  uncontested one — a terminal pane is by definition where untrusted third-party
  binaries run, every descendant inherits, and `/proc/PID/environ` is same-uid
  readable. Prefer fd/socket handoff or a 0600 file whose path is passed
  explicitly. (argv is *worse* than env — world-readable via `ps`.)
- **Mechanism note**: Rust's `Command::env_clear()` is the documented idiom, but on
  Windows it strips `SystemRoot` and many programs then fail to start
  (rust-lang/rust#114737); this repo already hit that
  (`docs/retro/2026-05-11-live-log-streaming-wrapper-failures.md:218`). So:
  allowlist on Unix, allowlist-plus-forced-system-vars on Windows. Must-preserve
  includes `PATH`, `HOME`, `USER`, `SHELL`, `TERM`, `TERMINFO`, `COLORTERM`,
  `LANG`/`LC_*`, `TZ`, `TMPDIR`, `XDG_*`, `DISPLAY`, `WAYLAND_DISPLAY`,
  `XAUTHORITY`, `DBUS_SESSION_BUS_ADDRESS`, `SSH_AUTH_SOCK`, proxy vars, `SHLVL`.

## 9. Recommended remediation

Ranked, and deliberately separable.

1. **Answer the question from 2026-06-12.** Classify every `AGENTMUX_*` var as
   identity/authority (never inheritable), behaviour flag (never inheritable), or
   helper-informational (inheritable). §6 is the proposed classification. This is
   the prerequisite for everything else.
2. **Sanitize sender-side at both pane-spawn boundaries** — an allowlist built by
   construction, not a denylist by subtraction. Snapshot the environment AgentMux
   was launched with *before* it sets any `AGENTMUX_*`, and build pane
   environments from that plus the explicit keep-set. This survives someone adding
   variable #21, which a denylist does not.
3. **Fix the sentinel gap**: set `AGENTMUX` on the agent-pane path too, or delete
   the dead `build_mux_env` and set it in one shared place. Make it
   self-describing (instance id + runtime dir + start time, tmux-style) so a stale
   inherited copy is detectable. PIDs are reused — include start time.
4. **Stop shipping the credential into panes.** Replace ambient
   `AGENTMUX_AUTH_KEY` with a per-invocation handoff: helpers discover the
   instance via a well-known per-instance socket under its own runtime dir and
   authenticate per call. If that is too large a change now, at minimum scope the
   key (capabilities, expiry) so an inherited copy is not omnipotent.
5. **Add an I7 invariant** — "an instance's identity must not be inheritable by
   processes it spawns" — with an enforcement test that launches a build from a
   pane and asserts it resolves its own channel and data dir. The absence of such
   a test is why five documents could identify this and none of them close it.

   **Landed in #3365**, with one deliberate substitution: the enforcement test
   checks *spawn-site coverage in source* (every `Command::new` either sanitizes
   or is exempt with a stated reason) rather than launching a real build. That
   targets the actual failure mode — both P0s on the original fix were a spawn
   path nobody applied the strip to — and runs in the existing per-PR lanes
   instead of needing a packaged artifact. The runtime check described above is
   still worth having and belongs in the nightly lane; it is not a substitute
   for this one, nor this for it. Auditing against the new invariant immediately
   found two live violations (`util.rs::open_browser`, `editor_handlers.rs`
   reveal-in-file-manager), both fixed in the same PR.
6. **Reconcile the three contradictory specs** (§7). **Done.** Only one of the
   three was actually wrong: `SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28`
   claimed the key is "not a PTY env var for trust reasons", sourced to
   `internals/env-vars.md` — a file that does not exist in this repo.
   `docs/MUXSH.md` and `docs/agent-identity-bootstrap.md` were both accurate:
   they describe agent panes, which is exactly where the key is injected. The
   stale paragraph now carries a dated correction rather than being deleted, so
   the next reader sees what changed and why.

## 10. What this does not explain

**None of this explains the vim freeze** that started the investigation. It
explains why *testing* that freeze was unreliable — builds silently running old
binaries and joining the wrong channel — but the freeze itself remains open. The
two are separate and should not be conflated in tracking.

## 11. Evidence appendix

Observed environment of an agent pane inside a running 0.56.2 instance
(`verify-decrqm-fix`), 2026-09-17:

```
AGENTMUX_AGENT_ID=Opaz
AGENTMUX_AGENTS_DIR=/home/yas/.agentmux/channels/verify-decrqm-fix/agents
AGENTMUX_APP_PATH=/tmp/.mount_AgentMGgcgie/usr/bin
AGENTMUX_AUTH_KEY=<redacted — equals that instance's authkey.dev auth_key>
AGENTMUX_BLOCKID=aa5437ec-3eb2-40bf-a5fd-e1ef54ec061d
AGENTMUX_CEF_CACHE_DIR=.../verify-decrqm-fix/versions/0.56.2/cef-cache
AGENTMUX_CHANNEL=verify-decrqm-fix
AGENTMUX_CONFIG_DIR=.../verify-decrqm-fix/config
AGENTMUX_CONFIG_HOME=.../verify-decrqm-fix/config
AGENTMUX_DATA_DIR=.../verify-decrqm-fix/versions/0.56.2/data
AGENTMUX_DATA_HOME=.../verify-decrqm-fix/versions/0.56.2/data
AGENTMUX_EXTRACTED_RUN=1
AGENTMUX_INSTANCE_DIR=.../verify-decrqm-fix
AGENTMUX_INSTANCE_RUNTIME_DIR=.../verify-decrqm-fix/versions/0.56.2/runtime
AGENTMUX_LOCAL_URL=http://127.0.0.1:33275
AGENTMUX_LOG_DIR=.../verify-decrqm-fix/versions/0.56.2/logs
AGENTMUX_RUNTIME_MODE=installed
AGENTMUX_SHARED_DIR=/home/yas/.agentmux/shared
AGENTMUX_SPLASH_READY_FILE=/tmp/agentmux-splash-ready-292324
AGENTMUX_SRV_PIPE_PATH=
```

`AGENTMUX_SRV_PIPE_PATH` being empty is itself worth noting: `srv_spawner.rs:449`
always sets it, and an empty inherited value can shadow an absent one in a
consumer that only tests presence.

Contamination caused by this investigation and not yet cleaned:
`channels/verify-decrqm-fix/versions/0.56.3/` (20 MB), created when a 0.56.3 build
launched from a pane inherited `AGENTMUX_CHANNEL=verify-decrqm-fix`.

## 12. Unresolved

- **Host-spawned sidecar srv** (`agentmux-cef/src/sidecar.rs:283-302`) inherits the
  host's environment, which carries `AGENTMUX_IPC_HASH`, `AGENTMUX_BACKEND_*`,
  `AGENTMUX_LAUNCHER_PIPE`, `AGENTMUX_INSTANCE_ID`. Whether that path is reachable
  in a shipped build was not traced; if it is, those vars reach panes too.
- **Windows/ConPTY** spawn path (`cmdbuilder.rs:646`) was not separately verified,
  though `portable_pty`'s env seeding is platform-independent.
