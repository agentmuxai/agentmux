# Retro: `package-portable.sh` wiped a running install

**Date:** 2026-05-28
**Severity:** High — visible to user as a 22-minute renderer crash loop in a long-running session.
**Affected:** 0.39.1 portable, any version using the unguarded `rm -rf` in `scripts/package-portable.sh`.
**Status:** Fix in this PR (package-portable guard). Two follow-up fixes tracked: `resolve_frontend_base_url` Result-typing, and `on_render_process_terminated` crash budget.

---

## What happened

A second AgentMux agent on the same machine invoked `bash scripts/package-portable.sh` with no output-dir argument while the user's portable install was running from the default location. The script's line 31:

```bash
OUTDIR="${1:-$HOME/Desktop}"
PORTABLE="$OUTDIR/agentmux-$VERSION-x64-portable"
…
# Clean previous
rm -rf "$PORTABLE" "$ZIPPATH"
```

resolved `$PORTABLE` to the live install's path and `rm -rf`ed it. NTFS lets you unlink mapped `.exe` / `.dll` files — the directory entries vanish, the file content survives only as long as the existing process keeps its handles open. So:

- The running app kept running on its mapped pages (the user noticed nothing).
- Every on-disk asset (`frontend/`, runtime `.pak`s, CEF DLLs, the launcher) was gone.
- Any code path that resolved an asset relative to `current_exe()` after the wipe got ENOENT.

The follow-up `package-portable.sh ~/Desktop/msix-build` invocation succeeded into the alternate directory, so the user ended up with the strange end-state of an *empty* `~/Desktop/agentmux-<version>-x64-portable/` next to a *full* `~/Desktop/msix-build/agentmux-<version>-x64-portable/`.

## Why it took ~4½ hours to manifest

The original window's frontend URL is passed by the launcher on the CLI — that resolution happens at launch, before the wipe, so the running window was unaffected. `resolve_frontend_base_url` is only consulted for **new** windows (pool warmups, tab tear-off, new top-level windows, floating panes). The user didn't open a new top-level window until 4½ hours later. The moment they did, `resolve_frontend_base_url`'s `frontend/index.html` existence check failed, the function fell back to `http://localhost:5173` (the dev Vite URL), the new renderer aborted, and the crash handler began an unbounded recovery-page loop (~108 events/sec for 22 minutes, 139k crashes total, 884 MB host log).

## Three independent gaps that compounded

| # | Gap | Where | Fix scope |
|---|---|---|---|
| 1 | `package-portable.sh` will wipe a running install | `scripts/package-portable.sh:31` | This PR |
| 2 | `resolve_frontend_base_url` silently degrades to a dev URL in production when assets are missing | `agentmux-cef/src/commands/window.rs:967` | Follow-up PR |
| 3 | `on_render_process_terminated` has no crash budget, so a single bad load loops forever | `agentmux-cef/src/client/mod.rs:1247` | Follow-up PR |

Any one of these three fixed in isolation would have downgraded this incident from "22-minute crash loop" to "single new-window failure with a clear error message". All three need fixing — they form a defense-in-depth chain, not redundant cover.

## This PR — Fix #1

`scripts/package-portable.sh` gets a pre-`rm` guard that uses PowerShell's `Get-Process` to detect any process whose `.Path` lives under the target portable directory. On hit:

```
ERROR: a process (PID <pid>) is running from <path>
       quit that AgentMux instance, or pass an alternate output dir:
       bash scripts/package-portable.sh ~/Desktop/staging
```

and exits 1 before the `rm -rf` runs. The guard only fires when `powershell.exe` is on PATH (i.e. Windows / Git-Bash environments), so non-Windows CI paths exercising this script are unaffected.

Verified against the surviving evidence from this incident:
- **Positive:** PID 719104 (the live launcher) is detected when checking `~/Desktop/agentmux-0.39.1-x64-portable/`. Guard would have fired.
- **Negative:** an older, idle `~/Desktop/agentmux-0.38.13-x64-portable/` on the same Desktop is not detected. Guard correctly skips.

## Follow-ups (not in this PR)

- **`resolve_frontend_base_url` returns `Result`** — never silently emit `localhost:5173` in production. Concrete signature + caller surgery in the forensic report.
- **Crash-budget on `on_render_process_terminated`** — per-browser 3-crashes-in-10s cap, then load a terminal static page instead of looping. Matches the prime directive of `docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` ("bounded recovery — never an infinite restart loop").
- **Log rate-limiting on `target = "crash"`** — 139k identical lines wrote 884 MB and CPU-pegged the host enough to starve the live renderer's IPC. Custom `tracing` layer or per-key emit-throttle.
- **Asset resolution off something stabler than `current_exe()`** — Windows's `GetModuleFileName` returns the pre-rename path forever, which is what made the wipe invisible to the running process. Anchor off `AGENTMUX_HOME` env (set by launcher) or an explicit marker walk instead.

## References

- `scripts/package-portable.sh` — the script with the unguarded `rm -rf`
- `agentmux-cef/src/commands/window.rs:967` — `resolve_frontend_base_url`
- `agentmux-cef/src/client/mod.rs:1247` — `on_render_process_terminated`
- `docs/specs/SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md` — shipped the recovery HTML; never added the crash budget
- `docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` — establishes "bounded recovery" as the prime directive that the current handler violates
- `docs/analysis/ANALYSIS_DEV_VITE_PORT_HARDCODE_2026-05-26.md` — earlier fix in the same function; left the production fallback intact
