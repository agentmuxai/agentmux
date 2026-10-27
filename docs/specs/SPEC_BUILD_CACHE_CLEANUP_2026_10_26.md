# Spec: Build-cache cleanup and a low-disk warning

**Status:** active — built in #5516 (Kanban #3664). Approved by the operator on 2026-10-25 ("use best recommendations"); defaults confirmed by the platform lead on 2026-10-26.
**Date:** 2026-10-26
**Author:** Loap-3
**Verified against:** `agentmux` `main` @ `2d8b80ed1`, and a read-only survey of one host's agent workspaces on 2026-10-26.

## 1. Ask

In two days three hosts nearly filled their disks from agents' cargo `target` folders; each time the agents cleared their own folders by hand (9 to 37 GB each). Clear them automatically, and warn when a disk runs low.

## 2. What fills the disk

A survey of one host's 16 clones (read-only):

- 1,020 GB of `target` folders. **682 GB (67%) is `target/<profile>/incremental`**, and 618 GB of that is the `agentmux_srv` crate.
- Why: rustc keeps one incremental directory per crate *and per hash of its inputs*. Every change to a dependency (`agentmux-common`, say) gives `agentmux_srv` a new directory of about 10 GB; the old one is never reused and cargo never collects it. 896 incremental crate directories, about 15 orphans of the srv crate per active clone.
- Age alone frees nothing: every clone had been built within three days. What frees space is *orphan* directories.

What each rule frees on that host, today:

| Rule | Frees |
|---|---|
| keep the newest 3 incremental dirs per crate and profile, remove the rest when idle 24 h | 296 GB (43% of incremental) |
| the same, idle 6 h | 397 GB (58%) |
| an agent finishes a task: clear that agent's `incremental` | that clone's whole incremental (25 to 155 GB each) |
| a clone not built in 3 days loses its `target` | 0 GB today; a net for abandoned clones |

The `[profile.dev] debug = "line-tables-only"` change (#5500) also makes new incremental directories smaller.

## 3. What is built

`backend/build_cache.rs` (file system and policy) and `backend/build_cache_service.rs` (schedule, warning, hook). None of it can break a build: the worst case of a wrong guess is a slower incremental rebuild.

1. **Orphans** (hourly, over every local agent workspace under `~/.agentmux/agents`, and when the disk is low): in each `incremental` folder, per crate, keep the newest `orphan_keep` (3) directories and remove older ones that have been idle `orphan_idle_hours` (24). A directory's age is the newest change in it or in the files two levels down, so a directory a build writes to is never old.
2. **A finished task**: when an agent completes a Kanban task (`POST /agentmux/work/:id/complete`), srv clears every `incremental` folder in that agent's own clones, in the background, when its workspace is under `~/.agentmux/agents` (an agent working in a project folder elsewhere is left alone). The completion never waits for it.
3. **Abandoned clones**: when a task finishes, or the disk is low, a clone with no build activity for `stale_days` (3) loses its whole `target`. The hourly pass does not do this.
4. **Never under a running build**: a clone with a cargo, rustc, linker or other build process, or a program run from its `target`, in it (by working directory, executable or command line) is skipped and the pass says so.
5. **A log** of every removal, one line each (time, reason, workspace, kind, bytes, path), in `~/.agentmux/logs/build-cache-cleanup.log`, and in srv's own log.
6. **The disk check**, every 5 minutes, on the disk that holds `~/.agentmux/agents`. When the free space is below `warn_free_gb` (20 GB) or `warn_free_pct` (10%) of the disk, a cleanup pass runs at once. If it is still low, the owners of the three largest workspaces and their leaders (the seat's `lead`) get an `[AgentMux]` note naming the largest build folders, with sizes and the part that is incremental. At most one warning an hour per host: a file in `~/.agentmux/shared/locks/` keeps the time, so two AgentMux channels do not both warn. A warning nobody received does not use the hour up, but the walk over every build folder that prepares it waits 30 minutes before the next try.

Only an srv started by its launcher on the real home cleans (Kanban #4534). `~/.agentmux/agents` is shared by every AgentMux on the host, so a `task dev` build, a headless srv, a binary run from a source-tree output folder or an isolated instance (`AGENTMUX_HOME_OVERRIDE`) would otherwise prune the real folder too: on one host a headless srv removed a stale target that way. srv decides from how it was started: it cleans when it has a channel that is not `dev-*` (the installed release's `stable`, or a local package's `local-*`: a host that runs a local package has no `stable` srv, and then nothing else would ever clean), its binary is not in a source-tree output folder, the runtime mode is not `dev`, it is not headless and has no home of its own. Anything else schedules nothing, cleans nothing and sends no warning. `AGENTMUX_BUILD_CACHE_CLEAN` overrides it: `0` never cleans; `1` cleans (a headless host, say), except an instance with a home of its own. Of the srvs that may clean, one at a time does: each pass takes a host-wide lock (`~/.agentmux/shared/locks/build-cache-clean.lock`) and a pass that finds it held is skipped, since the other's pass covers the same folders.

## 4. Settings

In the settings file's free-form keys; a missing or mistyped value keeps its default.

| Key | Default | Meaning |
|---|---|---|
| `buildcache:enabled` | `true` | Everything but the warning |
| `buildcache:orphans` | `true` | Orphan pruning (hourly and low-disk) |
| `buildcache:orphan_keep` | `3` | Newest incremental dirs kept per crate and profile (at least 1) |
| `buildcache:orphan_idle_hours` | `24` | Idle time before an extra directory goes |
| `buildcache:stale_days` | `3` | A clone with no build for this long loses its target (0 = off) |
| `buildcache:warn_free_gb` | `20` | Warn below this many GB free |
| `buildcache:warn_free_pct` | `10` | ...or below this percentage free |

## 5. Tests

`build_cache_tests.rs`: policy defaults and overrides; low by bytes or by percentage; clone discovery; incremental folders for profiles and cross builds; the orphan rule (all but the newest few per crate, only when idle, a rewritten directory counts as new); a finished task clears incremental, the hourly pass prunes orphans, both log the workspace; a quiet clone loses its target on a finished task or low disk and not hourly, and the setting 0 turns it off; a clone with a build in it is skipped (and our own test binary shows the process check sees a program run from a folder); sizes and units; the disk of a path. `build_cache_service.rs`: who hears of a warning (top three owners and their leader, once each); the note's text; one warning an hour across channels. An ignored test prints what the orphan rule would remove from a real folder.

## 6. Not done

- Other hosts' disks are handled by their own srv; nothing here reaches across hosts.
- A clone outside `~/.agentmux/agents` (a shared checkout) is not looked at.
- The count of incremental directories kept is the same for every crate; the `agentmux_srv` crate, 90% of the weight here, could use a lower number (it has the most variants: bin, test, integration tests).
