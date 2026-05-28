---
type: patch
---

fix(package-portable): refuse to wipe a running install

`scripts/package-portable.sh:31` did an unconditional `rm -rf "$PORTABLE"`.
With no output-dir arg, `$PORTABLE` defaults to
`$HOME/Desktop/agentmux-<version>-x64-portable/` — the canonical location a
portable user runs from. NTFS unlinks happily through mapped exe/dll files,
so the script silently deleted the on-disk assets of a live instance; the
process kept running on its mapped pages but every `current_exe()`-relative
asset lookup (notably `frontend/index.html` for `resolve_frontend_base_url`)
returned ENOENT on the next new-window open, leading to a 139k-event
renderer crash loop.

Add a pre-`rm` guard that uses PowerShell's `Get-Process` to detect any
process whose `.Path` lives under `$PORTABLE`. On hit, bail with a clear
remediation hint pointing at the alternate-output-dir form. Negative case
(non-running portable) verified against an idle older portable on the same
Desktop.

The guard only runs when PowerShell is available, so non-Windows CI paths
that exercise this script are unaffected.
