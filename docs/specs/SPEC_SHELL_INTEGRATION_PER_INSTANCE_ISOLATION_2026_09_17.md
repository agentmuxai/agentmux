# SPEC: Shell-integration scripts must be per-instance, not machine-global

**Author:** Opaz
**Date:** 2026-09-17
**Status:** implemented

---

## 1. Problem

Shell-integration scripts (the rcfiles every interactive terminal sources, plus
`muxlog.mjs` / `muxspect.mjs` / `muxopen.mjs` / `muxsh.mjs`) deploy to a single
machine-global directory — `~/.agentmux/shell/` — shared by **every instance of
every version** on the machine, dev and packaged alike.

Redeployment is gated on `~/.agentmux/shell/.version`, whose marker is the app
version plus a hash of the script contents (`version_marker()`). Two
concurrently-running versions therefore have different markers and each
overwrites the other's scripts.

This is not a boot-time event that settles. `deploy_scripts` is called from two
places: `bootstrap.rs` once per srv start, **and
`blockcontroller/shell/lifecycle.rs` on every interactive shell spawn**. So the
rewrite happens every time anyone opens a terminal, in any instance, forever.

### 1.1 Demonstrated live (2026-09-16)

Watching `~/.agentmux/shell/.version` while starting an instance in a
**completely separate channel**:

| Time | `.version` | Trigger |
|---|---|---|
| 20:37:20 | `0.56.3-94ef97c98e76de45` | a 0.56.3 instance booted |
| 23:49:19 | `0.56.2-a8a36d037c528879` | a 0.56.2 instance booted in channel `opaz-iso562` |
| 23:49:46 | `0.56.3-94ef97c98e76de45` | **one terminal opened** in the 0.56.3 instance flipped it back |

27 seconds apart. Nothing here is a race or a corner case — it is the designed
behaviour of a version-keyed marker over an unkeyed shared path.

### 1.2 Why it matters

- **Violates I6** (CLAUDE.md): *instances of different `(channel, version)` never
  share a data/logs/cef-cache directory.* Data dirs are carefully isolated per
  build; the scripts every terminal sources at startup then silently are not.
- **Torn reads.** `std::fs::write` truncates before writing, so there is a
  window where a shell starting in instance A sources a `.bashrc` that instance
  B is midway through replacing.
- **Protocol skew.** The `.mjs` helpers talk to a backend over its API, so a
  0.56.2 instance can end up running 0.56.3's `muxlog.mjs` against a 0.56.2 srv.

## 2. Constraint that rules out the obvious fix

"Move it under the instance data dir" is wrong, and the existing comment in
`lifecycle.rs` says why:

> MSIX packages virtualise writes to `%LocalAppData%`, so files written by the
> packaged backend aren't visible to child processes (pwsh, bash, etc.) spawned
> via ConPTY. The home dir is never virtualised, so the scripts are always
> reachable at their literal path.

The global path is deliberate. Any fix must keep the scripts under a
never-virtualised home path.

## 3. Second constraint: a documented user-facing path

`docs/MUXSPECT.md` and `docs/MUXSH.md` document invoking the helpers directly:

```
node ~/.agentmux/shell/muxspect.mjs list
```

That path must keep working and must stay *predictable* — a user cannot know
their instance's hash. So it cannot simply move.

## 4. Design

Keep the home base; key the subdirectory, mirroring what **I5** already requires
of every named OS object.

- **Per-instance root** — `~/.agentmux/shell/<key>/`, where `<key>` is a 16-hex
  FNV-1a hash of the canonicalised, lowercased instance data dir. The data dir
  is `<channel>/versions/<version>/data` by construction, so it already
  distinguishes exactly the pairs I6 names. FNV-1a (not `DefaultHasher`) because
  this names a directory that must be stable across runs.
  **This is the root the rcfiles are sourced from.**
- **Documented root** — `~/.agentmux/shell/` keeps being deployed, shared and
  last-writer-wins, purely so §3's direct-invocation path keeps working. Nothing
  sources it at shell startup.

`deploy_scripts` and `get_shell_startup` now both take the shell root directly
(previously each appended `"shell"` internally), and both call sites obtain it
from one helper — so they cannot drift apart.

### 4.1 Ordering hazard, recorded

`integration_base()` resolves the data dir via `get_mux_data_dir()`, which reads
`AGENTMUX_DATA_HOME`. `bootstrap::open_stores_and_migrate` overwrites that from
the resolved `config.data_home`; **before it runs, the variable can still hold a
value inherited from whichever instance spawned this process.** Observed live: a
dev srv launched from a terminal inside a 0.56.2 instance carried that
instance's data dir in its environment. Both call sites run after bootstrap, so
both see the corrected value; calling `integration_base()` earlier would not be
safe, and the function's doc comment says so.

## 5. Tests

Both load-bearing tests were confirmed failing before the fix:

- `two_instances_never_share_a_shell_integration_dir` — same channel/different
  version, and same version/different channel, must not collide. Against the
  pre-fix unkeyed path it fails with `left: "/home/u/.agentmux"`,
  `right: "/home/u/.agentmux"` — the bug itself.
- `the_deployed_rcfile_is_exactly_the_one_the_shell_is_told_to_load` — resolves
  the `--rcfile` argument to a real file on disk and checks it contains the
  integration script. This is the coupling that actually matters: if deploy and
  spawn drift, scripts land where nothing sources them and every terminal
  silently loses integration with no error anywhere. Falsified by introducing
  deliberate drift, which produces
  `shell is told to load …/shell/shell/bash/.bashrc but deploy_scripts never wrote it`.
  Worth noting the pre-existing `test_bash_startup_args` does **not** catch that
  drift — it only asserts `contains("bash")` and `ends_with(".bashrc")`, so it
  passes both before and after a path-contract change.
- `the_same_instance_always_resolves_to_the_same_dir` — the path is recomputed on
  every shell spawn, so an unstable key would send a later shell to an empty dir.
- `deployment_stays_under_the_never_virtualised_home_dir` — guards §2.

## 6. Out of scope

- **Stale directory sweep.** Every local build gets its own channel, so keyed
  dirs accumulate one per `(channel, version)` ever run (~100 KB each). Bounded
  and small, but unbounded over a developer's lifetime. Deliberately not solved
  here; it needs a liveness rule, which is its own decision.
- **The documented root still flip-flops** between versions (§4). It is only
  reached by explicit manual invocation, not by shell startup, so the blast
  radius is a user running one ad-hoc command against the wrong version's
  helper. Fixing it means changing a documented path.
- **This does not fix the vim/terminal-lock report** it was found while
  investigating (`docs/reports/REPORT_VIM_PANE_LOCK_0562_VS_0563_2026_09_16.md`).
  The deployed `.bashrc` defines on-demand functions and a prompt-time OSC 7
  hook, so there is no mid-alternate-screen write path. This is fixed on its own
  merits as an isolation defect.
