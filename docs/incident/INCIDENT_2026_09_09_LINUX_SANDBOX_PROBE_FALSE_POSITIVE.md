# Incident: Linux userns-sandbox probe reports "available" when AppArmor blocks it

**Status:** implemented — fix in PR #3117 (`run_internal_userns_probe_and_exit`, `agentmux-cef/src/linux_sandbox.rs`). Not yet verified end-to-end on the affected VM (blocked on a CI-built artifact).
**Date:** 2026-09-09
**Component:** `agentmux-cef/src/linux_sandbox.rs` (landed via PR #2783, 2026-08-24)
**Symptom:** On an AppArmor-restricted Ubuntu host with no `agentmux-userns` profile
installed yet, launching AgentMux never shows the "sandbox blocked by system policy"
recovery dialog. Instead the CEF host process crash-loops every ~1.2s, each attempt
killed by Chromium's own hard check:

```
[pid:pid:...] FATAL:content/browser/zygote_host/zygote_host_impl_linux.cc:128]
No usable sandbox! If you are running on Ubuntu 23.10+ or another Linux distro
that has disabled unprivileged user namespaces with AppArmor, ...
```

## Root cause

`probe_userns_available()` re-execs the current binary with
`--internal-probe-userns`, which calls `run_internal_userns_probe_and_exit()`. That
function used to do exactly one thing: call `unshare(CLONE_NEWUSER)` and exit 0 if
the syscall returned 0.

That syscall **always returns 0** under Ubuntu's `apparmor_restrict_unprivileged_userns`
policy, even when the sandbox will not actually work. Confirmed directly on the
affected VM via `auditd`:

```
apparmor="AUDIT" operation="userns_create" class="namespace" \
  info="Userns create - transitioning profile" profile="unconfined" \
  comm="agentmux-cef" requested="userns_create" target="unprivileged_userns"

apparmor="DENIED" operation="capable" class="cap" \
  profile="unprivileged_userns" comm="agentmux-cef" \
  capability=21 capname="sys_admin"
```

`unshare(CLONE_NEWUSER)` itself is never denied — the process is transitioned into a
synthetic, restrictive `unprivileged_userns` AppArmor profile *inside* the new
namespace, and *that* profile denies `CAP_SYS_ADMIN`. Writing `uid_map`/`gid_map`
(and CEF's subsequent mount/pivot_root calls for its own sandbox) require
`CAP_SYS_ADMIN` inside the new namespace — exactly the operation the probe never
attempted. Reproduced directly:

```
$ unshare --user --map-root-user echo ok
unshare: write failed /proc/self/uid_map: Operation not permitted

$ python3 -c 'import ctypes,os; libc=ctypes.CDLL(None,use_errno=True); \
    print(libc.unshare(0x10000000)); open("/proc/self/uid_map","w").write(f"0 {os.getuid()} 1")'
0
# uid_map write raises PermissionError
```

So the probe reported "available" (false positive) on every launch, the `while
!probe_userns_available()` loop in `lib.rs::run()` never entered its body, no dialog
ever appeared, and CEF proceeded straight into its real (blocked) sandbox init.

Confirmed this wasn't a build/wiring issue first: the `sandbox` feature is on by
default and stays on with `cargo build --features patched-libcef` (features are
additive, not replacing); `strings` on the shipped binary shows both the pure
profile-text constants and the feature-gated dialog strings ("sandbox fix
installed", "no dialog tool"); `AGENTMUX_UNSAFE_NOSANDBOX` is unset in the real
launch environment; the launcher forwards no `--type=`-prefixed argument to the
top-level host spawn (`agentmux-launcher/src/host_spawn.rs::spawn_host_unix` only
forwards the launcher's own argv, which is empty on a normal desktop launch) so
`is_subprocess` is correctly `false` for the browser process.

## Fix

`run_internal_userns_probe_and_exit()` now writes `uid_map` after a successful
`unshare(CLONE_NEWUSER)`, and only exits 0 if that write also succeeds — testing the
capability AppArmor's policy actually gates, not just namespace creation. This
correctly flips `probe_userns_available()` to `false` on affected hosts, which
routes through the existing (already-implemented, already-tested) dialog → pkexec
one-time-fix → relaunch flow instead of falling through to Chromium's crash.

No change to the AppArmor profile itself (`build_apparmor_profile()`) — granting
`userns,` to the confined binary makes it keep its normal (unconfined) capability
set inside the new namespace instead of being downgraded to the restrictive
synthetic profile, which is the correct fix and was already implemented correctly.
Only the *detection* was wrong.
