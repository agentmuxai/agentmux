---
type: patch
---

fix(launcher,cef): anchor asset lookup on AGENTMUX_HOME env from launcher instead of fragile current_exe()

Windows's `GetModuleFileName` (called by `std::env::current_exe`)
keeps returning the path the .exe was *originally loaded from*, even
after that directory is renamed or unlinked. The 2026-05-28 incident
exploited this: an external `rm -rf` of the running portable's
directory left `current_exe()` pointing at an empty path, so
`current_exe().parent().join("frontend/index.html")` returned ENOENT,
which (pre-#1119) caused a silent fallback to `localhost:5173` and a
renderer crash loop. #1119 + #1120 + #1121 catch the symptom; this
PR removes the root architectural hazard.

`agentmux-launcher` already resolves `real_exe` from its own
`current_exe()` BEFORE spawning the host — that resolution path
walks from the launcher's stable on-disk location to the runtime
dir, so it always points at the directory that actually contains
the binaries. Export that resolved path to the host as
`AGENTMUX_HOME`.

`agentmux-cef`'s `resolve_frontend_base_url` (and `RuntimeMode`
detection) now prefer `AGENTMUX_HOME` over `current_exe().parent()`.
A new `resolve_host_runtime_dir()` helper centralises the
preference chain: env var first, `current_exe()` fallback for dev
mode / standalone invocations where the launcher isn't present.

Fallback to `current_exe()` is preserved so:
- `task dev` (which can invoke the host directly on Linux/macOS)
  keeps working without the launcher.
- Existing standalone smoke tests stay green.
- Pre-AGENTMUX_HOME launcher builds running against a newer host
  degrade gracefully (only the rename-hazard reappears, which the
  earlier PRs catch).

Closes one item of #1117. Composes with #1115, #1119, #1120, #1121.
