# Retro: `task dev` windows showed a cog in the Linux taskbar

**Date:** 2026-10-11
**Author:** Opaz
**Status:** implemented — `task dev` names its `.desktop` file and its windows'
app_id from one value, `AGENTMUX_DEV_APP_ID`, and warns when `task` runs as a snap
**Severity:** Low — cosmetic, Linux `task dev` builds only; packaged builds,
Windows and macOS were unaffected

---

## 1. Symptom

On GNOME 50 (Wayland, Ubuntu), every window of a `task dev` build showed a
generic cog in the dock instead of the AgentMux logo. Packaged builds (AppImage,
.deb, .rpm) on the same machine showed the logo, as did dev builds on Windows
and macOS. Linux dev windows had shown the logo before mid-September.

## 2. How GNOME picks the icon

GNOME Shell decides which app a window belongs to, and shows that app's icon. In
the order that matters here:

1. **Sandboxed apps.** If the window's process carries a snap's AppArmor label
   (or is a Flatpak), the window belongs to that snap or Flatpak, and nothing
   else. This is deliberate: a sandboxed program must not claim another app's
   identity by choosing its app_id.
2. **The app_id.** Otherwise the window's Wayland app_id (X11: `WM_CLASS`) is
   matched to a `.desktop` file of that name, or with that `StartupWMClass`.
3. With no match, the window gets a generic icon: the cog.

AgentMux sets the app_id itself (`window_settings.rs::linux_app_id()`):
`agentmux-<AGENTMUX_CHANNEL>-<version>`, so that two builds running side by side
get separate dock entries (SPEC_LINUX_DOCK_INSTANCE_GROUPING_2026_09_17, #3323).
Each launch path installs a `.desktop` file under the same name. Packaged builds
get both names from the build's baked `CHANNEL` file, the same source the launcher
uses. `task dev` computed the name in `Taskfile.yml`.

## 3. Root causes

There were two, and either alone gives the cog.

### 3.1 The dev app_id never matched its `.desktop` file (our bug, since #3323)

`Taskfile.yml` exported `AGENTMUX_CHANNEL=dev-<branch slug>`, installed
`agentmux-dev-<slug>-<version>.desktop`, and expected the host it starts next to
inherit the variable. It didn't. `task dev` starts the host through the launcher,
and the launcher gives every process it starts its own environment,
`DataPaths::to_env_vars()` (`crates/common/src/data_paths.rs`), including
`AGENTMUX_CHANNEL` set to the launcher's dev channel, `dev-<branch>-<clone id>`
(#1053). The slug also turned git's trailing newline into a dash.

Measured on a live dev host (branch `main`, 0.59.20):

| | value |
|---|---|
| host's `AGENTMUX_CHANNEL` | `dev-main-c8fd090d2b4adf1d` |
| host's app_id | `agentmux-dev-main-c8fd090d2b4adf1d-0.59.20` |
| `.desktop` file `task dev` installed | `agentmux-dev-main--0.59.20.desktop` |

### 3.2 A snap-installed `task` claims every dev window (environment)

On this machine `task` is a classic snap. A snap's AppArmor label is inherited by
every process it starts, so the dev launcher, host and srv all ran as
`snap.task.task (complain)`, with the snap's `SNAP_*` environment. GNOME
therefore treated every dev window as the `task` snap's (step 1 above), and that
snap has no `.desktop` file. Even with 3.1 fixed, and the window advertising
exactly the installed file's name, the dock showed the cog.

The label shows up in complain mode for a classic snap, which suggests snapd now
attaches a profile to classic snaps where it used to leave them unconfined. On
this machine the snapd snap moved to 2.77.1 on 2026-10-03, and `task` to rev 292
on 2026-10-01. We didn't test older versions to pin down which change did it.

## 4. How it was found

Each step was checked against a live dev host and a screenshot of the dock,
because reading the code had already been wrong once (§5).

1. The host's `/proc/<pid>/environ` and the installed file gave the 3.1 mismatch.
2. With 3.1 fixed, `WAYLAND_DEBUG=client` showed the window sending exactly the
   installed file's name (`set_app_id("agentmux-dev-…-0.59.20")`), yet the dock
   still showed the cog. GIO loaded the `.desktop` file without complaint.
3. Starting the same build outside the snap's cgroup scope didn't help: an
   anonymous `run-*.scope` and an XDG-named `app-agentmux-<id>-<n>.scope` both
   still gave the cog. So the scope was not the deciding factor.
4. The host's `/proc/<pid>/attr/current` was `snap.task.task (complain)`.
   Running `task`'s binary directly (`/snap/task/current/task dev`, which skips
   the snap launcher) gave an unlabelled host, and the dock showed the logo.

## 5. Why it was missed

- #3323's spec (§3.3) and the Taskfile comment both state that the launcher/host
  exec "later in this same script inherits" the exported channel. That holds for
  a plain environment variable, not for one the launcher rewrites. Nobody checked
  the variable in a running host, or looked at the dock.
- Review on #3323 caught two earlier mismatches, each by reading code. The fix
  for the second was also judged by reading.
- Nothing fails: everything works except the icon, and only on Linux in dev.
- 3.2 came from outside the repository, after a snapd or `task` refresh, and it
  hides 3.1: fixing either alone changes nothing on a machine with both.

## 6. Fix

- **3.1:** `task dev` computes the dev app_id once, `agentmux-dev-<slug>-<version>`,
  with git's newline stripped. It uses that value both as the `.desktop` file's
  name and as `AGENTMUX_DEV_APP_ID`, which the launcher leaves alone and the host
  uses as-is in `linux_app_id()`. The host honors it only in a dev build, judged
  by where the binary is (`agentmux_common::is_dev_build_exe`), not by the
  environment, so a packaged build started from a dev instance's pane can't adopt
  the dev window's name. Implausible values are ignored. Unit tests cover it.
- **3.2:** nothing inside `task dev` can drop an inherited AppArmor label, and
  GNOME 50 offers no protocol for a window to supply its own dock icon. So
  `task dev` detects the label (`/proc/self/attr/current` starting `snap.`) and
  says so: dev windows will show a generic icon; install `task` outside snap.

## 7. Verification

| `task` | app_id fix | host's AppArmor label | dock |
|---|---|---|---|
| snap | no | `snap.task.task (complain)` | cog |
| snap | yes | `snap.task.task (complain)` | cog |
| run directly (not via snap) | yes | unconfined | **AgentMux logo** |

## 8. Lessons

- **A variable exported before a launcher is not a variable the child sees.**
  When a supervisor builds its children's environment, check the value in the
  running child (`/proc/<pid>/environ`), not in the script.
- **Derive both sides of a name match from one value.** The packaged paths work
  because AppRun and the launcher read the same baked file. The dev path
  re-implemented the channel rule in shell, and the two diverged.
- **Check cosmetic integrations by looking.** A dock icon is verified by a
  screenshot of the dock. Two plausible code-level explanations here (the scope,
  then a reload race) were each disproved by one.
- **The process's identity matters to the desktop, not just the window's.** On
  GNOME, the AppArmor label and the systemd unit of a window's process can
  outrank what the window says about itself.
