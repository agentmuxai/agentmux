---
type: patch
---

fix(scripts): stop silently downgrading from GitHub App to PAT on Windows

Any agent whose environment had `MSYS_NO_PATHCONV=1` set authenticated with a
long-lived PAT instead of its GitHub App, on every call, with no warning.
`github-app-token.py` signalled "no App identity" with exit 2, which is also
what CPython returns when it cannot open the script file -- and under
Git-Bash with path conversion disabled, `python3.exe` received `/c/Users/...`,
resolved it to `C:\c\Users\...`, and exited 2. Callers read that as "no App
identity", stayed quiet by design, and fell through to the PAT tier.

`NoAppIdentity` now exits 3, callers resolve the script path with `cygpath -w`
when available, and exit 2 is reported loudly as "the interpreter never ran
the script".
