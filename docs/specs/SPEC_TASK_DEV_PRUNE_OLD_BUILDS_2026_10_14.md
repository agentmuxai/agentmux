# SPEC: `task dev` removes its old builds once nothing runs from them

**Date:** 2026-10-14
**Status:** implemented — in PR #4856.
**Author:** AgentA (Area54), at the operator's request
**Affects:** `Taskfile.yml` (`dev:serve`), [`scripts/prune-dev-builds.sh`](../../scripts/prune-dev-builds.sh) (new), [`scripts/prune-dev-builds.test.sh`](../../scripts/prune-dev-builds.test.sh) (new), [`.github/workflows/ci-pr.yml`](../../.github/workflows/ci-pr.yml)

## 1. What happened

On Windows every `task dev` run makes a fresh `dist/cef-dev-<epoch>` folder of about 1 GB: the launcher, the app, CEF and srv copied side by side. Nothing removed them. On one host, 28 of them were 21 GB, and the drive was down to 14 GB free.

The Taskfile said why it didn't prune: `rm -rf` on a folder a live app runs from deletes the files that aren't locked (the MCP and bash helpers) and fails on the locked ones, which leaves the running session with broken tool paths. A cleanup on that host did exactly this, because its list of folders to keep carried a trailing slash from `ls -d` and the exclusion matched nothing. Windows does not stop you deleting a running program's files, so only a liveness check protects them.

## 2. Behaviour

[`scripts/prune-dev-builds.sh`](../../scripts/prune-dev-builds.sh) deletes `dist/cef-dev-*` folders, and only a folder that no process runs from:

1. It lists every running process's executable path, lowercased. If that list is empty (the query failed), it deletes nothing.
2. For each `dist/cef-dev-*` folder except the one named by `--keep`, it deletes the folder unless a path in the list starts with the folder's path plus a backslash (so `cef-dev-1` is not mistaken for `cef-dev-10`). A clone built on a `subst` drive (the `MAX_PATH` workaround in the run skill) has two spellings: a program started through `Z:` reports `Z:\…`, while the script may hold the physical path behind it. The script checks the folder under both spellings, using `subst`'s own mapping list, in either direction.
3. A folder that is still in use after `--wait <seconds>` stays; the next run's prune takes it.
4. A folder it couldn't fully delete (a process started between the check and the delete) is reported and left.

`task dev` runs it twice, on Windows only (elsewhere it uses one fixed folder and reuses it):

- **At the start**, with `--keep` set to the folder it is about to create and `--grace 10`. This clears builds left by a session that was killed before it could clean up, which is the usual way one is left behind: the task is stopped from outside and never reaches its end. The grace skips any build modified in the last 10 minutes: a second `task dev` in the same clone may have just built its folder and not yet started the app from it, so no process runs from it yet.
- **After the launcher exits**, with `--only <this run's folder> --wait 20`: its own build and nothing else, so it can never delete another run's folder. The app's helper processes follow the launcher out within a few seconds. The task's exit status stays the launcher's: the launcher runs under `|| launcher_rc=$?` because a task block stops at its first failing command, which would skip the cleanup after a crash.

It only ever touches folders named `cef-dev-<something>` directly under this clone's `dist/`. `dist/cef`, `dist/bin` and the Unix `dist/cef-dev` are never candidates. `task clean:host` still removes everything and remains the manual option.

## 3. Not done here

- **Dev data folders.** Each branch's dev instance keeps its data in `~/.agentmux/dev/<agent>-<branch>`, about 0.2–1 GB each. That is the instance's saved state (its session persists across rebuilds), so deleting it is a different decision from deleting a build. They are still cleaned by hand (the disk-space runbook, §4.1); an age-based prune is a possible next step.
- **Vite.** The Vite server `task dev` starts in the background outlives the task when the launcher exits normally. It holds a port and a Node process but no disk.
- **`task package` output.** Portable builds go to the Desktop on purpose and are the operator's to remove.

## 4. Tests

[`scripts/prune-dev-builds.test.sh`](../../scripts/prune-dev-builds.test.sh) runs in CI against fake build folders and a fake process list: idle builds are deleted, the `--keep` build and a build with a running process are left whole, a similar folder name doesn't protect an idle build, an unreadable process list deletes nothing, `--dry-run` deletes nothing, and unrelated folders are never touched, `--only` deletes just the named build (and refuses names that aren't a `cef-dev-*` folder name), and `--grace` spares a fresh build but not the `--only` one. It was also run by hand against a real running program on Windows (a copy of `ping.exe` started from one fake build folder): the idle folder was deleted, the one with the running program stayed whole, including through `--wait 3`, and was deleted once the program exited. The same was done through a real `subst Z:` drive: the program reports `Z:\dist\…` while the script holds the physical path, and the build stays whole when the script is given either spelling.
