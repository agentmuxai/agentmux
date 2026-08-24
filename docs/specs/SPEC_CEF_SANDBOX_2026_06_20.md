# SPEC: Enable CEF Renderer Sandbox
**Status:** Approved for implementation
**Issue:** #1374
**Date:** 2026-06-20
**Author:** AgentMux Engineering

---

## 1. Problem

`no_sandbox: 1` is hardcoded unconditionally in the `Settings` struct at
`agentmux-cef/src/main.rs:665`. This disables the Chromium renderer sandbox on
every platform in every build (debug, release, portable). A compromised renderer
process — e.g. via a malicious URL loaded in a browser pane — runs with the same
OS privileges as the host, which inherits the shell environment: `AWS_SECRET_*`,
`GITHUB_TOKEN`, `AGENTMUX_AUTH_KEY`, full filesystem access. `CEF_ARCHITECTURE.md:975`
already documents the risk explicitly.

Browser panes accept arbitrary URLs (`browser_pane_navigate` has no allowlist),
so the threat is real and the blast radius is maximal.

This compounds PR #1373 (`ipc_token` hardening): a sandbox escape sits in front
of the already-powerful local IPC surface.

---

## 2. Goals

- Remove `no_sandbox: 1` and enable the appropriate sandbox on each platform.
- Ship in three sequential PRs (one per platform tier) so the macOS/Linux fixes
  land quickly without waiting for the harder Windows work.
- Provide a runtime escape hatch (`AGENTMUX_UNSAFE_NOSANDBOX=1`) that can be set
  by operators for known-incompatible environments (e.g. nested VM, Docker), with
  a mandatory log warning.
- Keep each PR shippable and bisectable.

## 3. Non-goals

- URL allowlist for browser panes (tracked separately; useful defence-in-depth
  but orthogonal to sandbox).
- GPU process sandboxing hardening — that's independent of renderer sandbox.
- Changing the CEF version or the patched-libcef fork.

---

## 4. Background: how the cef-rs 148 sandbox works per platform

### 4.1 macOS

The cef-rs crate exposes `cef::sandbox::Sandbox` (in `sandbox.rs`). It
dynamically loads `libcef_sandbox.dylib` from the framework bundle and calls
`cef_sandbox_initialize(argc, argv)` before any other code in the subprocess
helper path. The resulting opaque context pointer must be kept alive until
program exit (the `Drop` impl calls `cef_sandbox_destroy`).

**What exists already:**
- Separate helper app bundle (`AgentMux Helper.app`) is set up and bundled ✓
- `resolve_browser_subprocess_path()` correctly points CEF at the helper ✓

**What is missing:**
- `cef::sandbox::Sandbox::new()` + `.initialize(&args)` called at the very start
  of the subprocess branch (before `execute_process`). Currently the subprocess
  branch passes `null_ptr` as `sandbox_info` to `execute_process` and
  `initialize`.

### 4.2 Linux

**2026-08-23 update:** "no setuid helper or extra init call is needed" below
was true on kernels ≥3.8 as originally understood, but not unconditionally —
Ubuntu (23.10, later backported to 22.04/20.04) added an AppArmor policy
that blocks unprivileged user-namespace creation by default, breaking the
namespace sandbox for any Chromium/CEF-based app system-wide unless the
binary has an explicit AppArmor exception. See
`docs/specs/SPEC_LINUX_SANDBOX_APPARMOR_USERNS_2026_08_23.md` for the
recovery mechanism (a one-time, `pkexec`-installed AppArmor exception,
scoped to survive AgentMux updates) — this doesn't change anything below,
just narrows the "no setup needed" framing.

Linux uses the Zygote process model with seccomp-BPF + kernel namespace
isolation. No setuid helper or extra init call is needed on kernels ≥3.8 when
the namespace sandbox is used. The correct flags are:

- `--disable-setuid-sandbox` — tells Chromium not to look for a setuid
  `chrome-sandbox` binary and fall back to the namespace sandbox instead.
- Do **not** pass `--no-sandbox` (that would disable all isolation).
- Do **not** pass `--no-zygote` (the Zygote handles sandboxed forking).

**What exists already:** nothing extra is needed in the Rust code beyond removing
`no_sandbox: 1` from `Settings`.

**What is missing:**
- `no_sandbox: 1` is set globally (removes all isolation).
- `--disable-setuid-sandbox` is not explicitly added to the CEF command line
  (should be added so Chromium doesn't abort looking for a missing SUID helper).

### 4.3 Windows

Windows sandbox is the most complex. The cef-rs 148 `sandbox` feature implements
the **DLL wrapper pattern**: the Rust crate is compiled as a `cdylib`, and the
CEF-provided `bootstrap.exe` acts as the real launcher — it initializes
`cef_sandbox_info_t` before any user code, then loads and calls into the DLL.

**What exists already:**
- `sandbox = ["cef/sandbox"]` feature defined in `agentmux-cef/Cargo.toml:16` ✓
- `bootstrap.exe` present in the CEF distribution ✓
- The `cef/build_util/win` bundle tool knows to copy `bootstrap.exe` and link
  against `cef_sandbox.lib` when `sandbox` feature is active ✓

**What is missing:**
- `[[lib]]` target with `crate-type = ["cdylib"]` in `agentmux-cef/Cargo.toml`
  (currently only a `[[bin]]` target exists).
- A `lib.rs` entry point that exports `DllMain` / the expected CEF bootstrap
  symbols so `bootstrap.exe` can find them.
- `bootstrap.exe` bundled into `dist/cef-dev/runtime/` and `task bundle` output.
- The entire host `main()` restructured to be callable as a DLL export.

This is effort-L and ships as a dedicated PR (Phase 3).

---

## 5. Implementation plan

### Phase 1 — macOS sandbox (this PR first)

**File: `agentmux-cef/src/main.rs`**

1. Add `#[cfg(all(target_os = "macos", feature = "sandbox"))]` import of
   `cef::sandbox::Sandbox`.

2. At the very top of `main()`, before `Args::new()`, initialize the sandbox
   context on macOS subprocess paths:

```rust
#[cfg(all(target_os = "macos", feature = "sandbox"))]
let mut _sandbox = {
    let mut s = cef::sandbox::Sandbox::new();
    // Must be called before execute_process.
    // Safe: Args::new() hasn't been called yet; no CEF heap in use.
    let args = cef::args::Args::new();
    s.initialize(args.as_main_args());
    s
};
```

3. Change `no_sandbox` to be conditional:

```rust
let settings = Settings {
    #[cfg(not(feature = "sandbox"))]
    no_sandbox: 1,
    // ...
};
```

4. Update `agentmux-cef/Cargo.toml` `[features]`:
   ```toml
   default = ["sandbox"]
   sandbox = ["cef/sandbox"]
   ```
   (Enable by default; `--no-default-features` opts out for special builds.)

5. Add escape hatch: if `AGENTMUX_UNSAFE_NOSANDBOX=1` is set at runtime, log a
   `tracing::warn!` and proceed with `no_sandbox: 1` regardless of feature flag.
   This is checked at runtime inside `main()` before `initialize()`.

**Taskfile / bundle:**

The macOS helper app bundle is already correct. No Taskfile change needed for
Phase 1.

**Cargo.toml:**

Add `libloading` to dependencies (required by `cef::sandbox::Sandbox` on macOS):
```toml
[target.'cfg(target_os = "macos")'.dependencies]
libloading = "0.8"
```

(Check if `cef/sandbox` feature already pulls it transitively — if so, omit.)

---

### Phase 2 — Linux sandbox (can land same PR as Phase 1 or separate)

**File: `agentmux-cef/src/app.rs`** (`on_before_command_line_processing`)

Add to the Linux section:

```rust
#[cfg(target_os = "linux")]
{
    // Use kernel namespace sandbox instead of setuid chrome-sandbox.
    // This works on kernels ≥3.8 without a privileged helper binary.
    cmd_line.append_switch(Some(&CefString::from("disable-setuid-sandbox")));
}
```

**File: `agentmux-cef/src/main.rs`**

Same `no_sandbox` conditional as Phase 1 (shared change).

**No Taskfile changes needed** — `chrome-sandbox` binary is not required when
namespace sandbox is active.

---

### Phase 3 — Windows sandbox (dedicated PR, effort-L)

**Overview:** restructure the crate to build as both `[[bin]]` (sandbox-off,
existing path) and `[[lib]]` (sandbox-on, new path). The DLL export surface
matches what `bootstrap.exe` calls.

**File: `agentmux-cef/Cargo.toml`**

```toml
[[bin]]
name = "agentmux-cef"
path = "src/main.rs"
# Built when sandbox feature is NOT active (all non-Windows or sandbox-off)

[lib]
name = "agentmux_cef"
path = "src/lib.rs"
crate-type = ["cdylib"]
# Built when sandbox feature IS active on Windows
```

**New file: `agentmux-cef/src/lib.rs`**

Export the `DllMain`-equivalent entry point that `bootstrap.exe` calls:

```rust
#[cfg(all(target_os = "windows", feature = "sandbox"))]
#[no_mangle]
pub extern "C" fn agentmux_cef_main(
    instance: *mut std::ffi::c_void,
    sandbox_info: *mut std::ffi::c_void,
) -> i32 {
    crate::main_impl(sandbox_info)
}
```

The existing `main()` body becomes `main_impl(sandbox_info: *mut c_void)` which
receives the already-initialized sandbox info from `bootstrap.exe`.

**Taskfile:** add `bootstrap.exe` copy step to `bundle:windows` and
`build:cef-dev` tasks.

**`cef_sandbox.lib` linking:** the `cef/sandbox` build_util feature already
handles this via `cargo:rustc-link-lib=static=cef_sandbox` in the build script.

---

## 6. Escape hatch

In `main()` (browser process path only), before `initialize()`:

```rust
let force_no_sandbox = std::env::var("AGENTMUX_UNSAFE_NOSANDBOX")
    .map(|v| v == "1")
    .unwrap_or(false);
if force_no_sandbox {
    tracing::warn!(
        "AGENTMUX_UNSAFE_NOSANDBOX=1 set — renderer sandbox DISABLED. \
         This should only be used in known-incompatible environments."
    );
    settings.no_sandbox = 1;
}
```

Document in `settings-template.jsonc` and `CEF_ARCHITECTURE.md`.

---

## 7. Testing

### Per platform

| Test | macOS | Linux | Windows |
|------|-------|-------|---------|
| App starts, browser pane renders | ✓ | ✓ | ✓ (Phase 3) |
| Renderer process visible in task manager with restricted token | - | - | ✓ |
| `chrome://sandbox` shows renderer Sandbox=Yes | ✓ | ✓ | ✓ |
| Agent panes, terminal panes unaffected | ✓ | ✓ | ✓ |
| Tear-off / float pane redock works | ✓ | ✓ | ✓ |
| `AGENTMUX_UNSAFE_NOSANDBOX=1` falls back gracefully | ✓ | ✓ | ✓ |
| `task dev` works without sandbox feature | ✓ | ✓ | ✓ |

### Regression signals

- GPU process crash (same symptoms as #778) — if sandbox forces GPU isolation
  that the driver doesn't support, browser panes go black. Mitigated by the
  existing `--disable-gpu` dev-mode flag (issue #778).
- Namespace sandbox failure on Linux (older kernel / Docker without user
  namespaces) — escape hatch covers this.

---

## 8. Rollout

- Phase 1+2 (macOS + Linux): ship together, `default = ["sandbox"]` means sandbox
  is ON by default in `task dev` and `task package`. If issues arise, set
  `AGENTMUX_UNSAFE_NOSANDBOX=1` at the OS level.
- Phase 3 (Windows): separate PR; sandbox OFF by default until tested end-to-end.
  Flip `default = ["sandbox"]` in a follow-up once Windows path is verified.

---

## 9. Files changed (Phases 1+2)

| File | Change |
|------|--------|
| `agentmux-cef/src/main.rs` | Sandbox init on macOS, conditional `no_sandbox`, escape hatch |
| `agentmux-cef/src/app.rs` | `--disable-setuid-sandbox` on Linux |
| `agentmux-cef/Cargo.toml` | `default = ["sandbox"]`, `libloading` dep (macOS) |
| `docs/CEF_ARCHITECTURE.md` | Document sandbox status + escape hatch |

## 10. Files changed (Phase 3, Windows)

| File | Change |
|------|--------|
| `agentmux-cef/Cargo.toml` | Add `[lib]` cdylib target |
| `agentmux-cef/src/lib.rs` | New — DLL export entry point |
| `agentmux-cef/src/main.rs` | Refactor body into `main_impl(sandbox_info)` |
| `Taskfile.yml` | Bundle `bootstrap.exe`, `agentmux_cef.dll` |
