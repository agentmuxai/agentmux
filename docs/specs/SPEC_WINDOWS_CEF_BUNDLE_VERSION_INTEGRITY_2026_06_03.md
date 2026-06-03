# SPEC: Windows CEF Bundle Version Integrity & Loud Startup Failure

**Date:** 2026-06-03
**Status:** Draft
**Author:** AgentX
**Related:** PR #1221 (CEF 146→148 bump), PR #1232 (deterministic bundle from `target/release` — *partial fix*), PR #1243, [[SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03]]

---

## 1. Symptom

A `task package` portable build `v0.42.0+g3012bfc5…` placed on the Desktop showed a
**splash screen, then nothing** — no window, no error, on every double-click. The only
trace was `runtime/debug.log`:

```
[0603/065525:ERROR:cef\libcef_dll\libcef_dll2.cc:92]
    Request for unsupported CEF API version 14800   (×8)
```

This is **not** single-instance behavior, not the user's double-click, and not a host
kill (see companion isolation spec). It is a **packaging-integrity defect**: the build
shipped a CEF runtime that does not match the host binary.

---

## 2. Root cause (evidence)

Repo `C:/Users/asafe/.claw/agentx-workspace/agentmux`, HEAD `3012bfc5`.

| Fact | Evidence |
|---|---|
| Host links against **CEF 148** | `agentmux-cef/Cargo.toml:33` → `cef = { version = "148", … }`; the host requests API `CEF_API_VERSION_LAST` = **14800** (`agentmux-cef/src/main.rs:216`) |
| Bundled runtime is **CEF 146** | Desktop `runtime/libcef.dll` ProductVersion `146.0.9 / chromium 146.0.7680.165`; `chrome_elf.dll` `146.0.7680.165` |
| `Cargo.lock` is consistent at 148 | both `cef` and `cef-dll-sys` resolve to `148.3.0+148.0.9` from crates.io — there is **no** `[patch.crates-io]` and **no** version skew in the lock |
| The 146 runtime comes from a **stale staging dir** | `target/cef-sdk/cef_windows_x86_64/libcef.dll` = CEF `146.0.9`, dated **Mar 27**, **262,352,384 bytes** — byte-identical to both `target/release/libcef.dll` and the Desktop portable's `runtime/libcef.dll` |
| Not a `CEF_PATH` override | `CEF_PATH` unset in build env; `.cargo/config.toml` has it only as a comment |

**Mechanism (empirically verified).** The `cef` crate is pulled with
`features = ["build-util"]` (`agentmux-cef/Cargo.toml:33`). build-util **stages a CEF
binary SDK into `target/cef-sdk/`** and **reuses an existing staging dir across crate
version bumps — the staging is NOT keyed on the CEF version.** `cef-dll-sys`'s build
script then copies that staged runtime into `target/release/`, and `bundle:windows`
(`Taskfile.yml`) copies `target/release/` into `dist/cef/` → the portable.

So when the crate was bumped 146→148 (#1221), the **March CEF-146 `target/cef-sdk/`
staging was never invalidated**; every subsequent build kept serving 146 into a
148-linked host. This was confirmed directly: removing only `target/release/libcef.dll`
+ the `cef-dll-sys-*` build dirs and doing a **full rebuild still reproduced the 146
DLL** (re-copied from the stale `target/cef-sdk/`); the runtime only corrects to 148
once `target/cef-sdk/` itself is removed and re-downloaded. PR #1232 made the bundle
*source path* deterministic but added **no version verification**, so a stale-staged
146 runtime sails straight through to the portable.

**Two independent defects compound:**
1. **No version guard** anywhere — nothing asserts `bundled libcef.dll major == linked
   cef crate major` (searched `Taskfile.yml`, `scripts/`, `build.rs` — none).
2. **Silent host failure** — on CEF init mismatch the host logs to
   `cef-debug.log` and `std::process::exit(exit_code)` with **no user-facing dialog**
   (`agentmux-cef/src/main.rs:700-736`), producing "splash then nothing."

---

## 3. Goals / Non-Goals

**Goals**
- G1. A build can **never silently ship** a `libcef.dll` whose major version differs
  from the linked `cef` crate. Bundling fails loudly instead.
- G2. The bundle sources CEF from a **guaranteed-fresh** location tied to the current
  build, not a possibly-stale shared dir.
- G3. If a version mismatch ever reaches a user anyway, the host shows a **clear,
  actionable error** (not a vanishing splash).
- G4. A regression test that fails if a mismatched runtime is bundled.

**Non-Goals**
- Changing the CEF version itself or the `cef`/`cef-dll-sys` crates.
- macOS/Linux bundling (their resolvers differ — `bundle:darwin`,
  `scripts/resolve-cef-runtime.sh`; apply the same *guard* idea as a follow-up).

---

## 4. Proposed changes

### 4.1 Version guard in `bundle:windows` (G1 — primary, highest ROI)
After confirming `libcef.dll` exists, assert its version matches the linked crate, and
**fail the build** on mismatch.

- **Expected major:** parse the linked `cef` crate version from `Cargo.lock`
  (the `[[package]] name = "cef-dll-sys"` / `name = "cef"` version → major, e.g. `148`).
- **Actual major:** read the bundled `libcef.dll`'s `ProductVersion` (e.g. via
  `(Get-Item …).VersionInfo.ProductVersion` → `146.0.9…` → major `146`). The build
  task already shells out; a small `pwsh -NoProfile` call is fine and deterministic.
- On mismatch (implemented in `scripts/verify-cef-version.sh`):
  ```
  ❌ CEF version mismatch: bundled libcef.dll is 146.0.9… (major 146) but the host
     links cef crate major 148. The runtime is stale relative to the linked bindings…
     Fix:  task clean:cef && task build:host
     See:  docs/specs/SPEC_WINDOWS_CEF_BUNDLE_VERSION_INTEGRITY_2026_06_03.md
  exit 1
  ```
This single guard turns the silent broken build into an immediate, self-explaining
failure — verified to fire on the exact 146-vs-148 case (and to pass once the runtime
is corrected to 148).

### 4.2 Invalidate the build-util staging on CEF change (G2 — robustness)
The runtime ultimately originates from `target/cef-sdk/` (the `cef` build-util staging),
which is **reused regardless of the crate version**. Sourcing the bundle from the
`cef-dll-sys` build-out dir does NOT help — that dir is itself populated from the same
stale staging. The durable fix is to keep the staging in sync with the linked crate:
- `clean:cef` (4.3) removes `target/cef-sdk/` so a version bump forces a re-download
  (the operational fix shipped here); and/or
- record the staged CEF version (e.g. a `target/cef-sdk/.cef-version` marker) and have
  `build:host` re-stage automatically when it differs from the linked crate version
  (durable follow-up — removes the manual `clean:cef` step entirely).
Keep 4.1 as the safety net regardless — even a "fresh" staging can be wrong.

### 4.3 `task clean:cef` helper (G2 — operational, shipped)
A targeted clean that removes the **build-util staging** (the actual culprit) plus the
downstream copies, so the next `task build:host` re-downloads CEF matching the linked
crate:
```yaml
clean:cef:
  cmds:
    - '{{.RMRF}} "target/cef-sdk"'                       # build-util staging (root cause)
    - '{{.RMRF}} "target/release/build/cef-dll-sys-*"'
    - '{{.RMRF}} "target/fast-release/build/cef-dll-sys-*"'
    - '{{.RMRF}} "target/release/libcef.dll"'            # downstream copies …
```
(Uses `{{.RMRF}}` so it works under the Taskfile shell on every OS.)

### 4.4 Loud host-side failure (G3 — UX backstop)
In `agentmux-cef/src/main.rs`, when `initialize()` returns non-success with an exit
code in the "real failure" branch (`main.rs:700-736`), on Windows show a
`MessageBoxW(MB_OK | MB_ICONERROR)` before exit:
> "AgentMux couldn't start its browser engine (CEF API version mismatch). This build's
> runtime is incompatible with the app binary — likely an incomplete build. See
> `…/logs/cef-debug.log`."
Keep it behind the same branch that already classifies non-clean exit codes (so normal
"process singleton" early-exits 0/24/36/38 stay silent). This converts "splash then
nothing" into a diagnosable message for any future slip. The `api_hash(...)` call at
`main.rs:216` is currently `let _ =` (fire-and-forget); consider checking its result to
fail *before* splash, pre-empting the wasted splash entirely.

### 4.5 Regression test (G4)
- A Windows CI test (or a `task` check) that, after `build:host` + `bundle`, runs the
  4.1 guard and asserts exit 0; and a negative test that drops a deliberately-wrong
  `libcef.dll` into `target/release/` and asserts the guard exits non-zero with the
  mismatch message.
- Optional smoke test: launch the packaged host headless and assert it reaches a
  "CEF initialized" log line within N seconds (catches *any* runtime breakage, not just
  version).

---

## 5. Immediate remediation (to unblock testing today)

Independent of the durable fixes, produce a working `v0.42.0` portable now:
1. `task clean:cef` — **must remove `target/cef-sdk/`** (the build-util staging), not
   just `target/release/`. Removing only the latter reproduces 146 from the stale
   staging (verified). The shipped `clean:cef` does this.
2. `task build:host` — with the staging gone, `cef` build-util re-downloads CEF **148**
   into a fresh `target/cef-sdk/` and `cef-dll-sys` copies it into `target/release/`.
3. Verify: `(Get-Item target/release/libcef.dll).VersionInfo.ProductVersion` starts
   with `148`, or run `bash scripts/verify-cef-version.sh target/release` (exit 0).
4. `task package` → new Desktop portable (the guard now runs during `bundle:windows`).
   Double-click should open a window, and per the isolation spec it runs fine alongside
   the existing instance.

---

## 6. Acceptance criteria

- AC1. With a stale/mismatched `target/release/libcef.dll`, `task package` **fails**
  with the §4.1 message (no portable produced).
- AC2. A clean build produces a portable whose `runtime/libcef.dll` major == the linked
  `cef` crate major.
- AC3. A host launched against a mismatched runtime shows the §4.4 dialog instead of a
  silent splash-then-exit.
- AC4. The §4.5 negative test fails the build deterministically.

## 7. Rollout

1. Land §4.1 guard + §4.3 `clean:cef` first (smallest, prevents recurrence immediately;
   feature PR + changeset, no version bump per `CLAUDE.md`).
2. Land §4.4 loud failure.
3. Land §4.2 fresh-source + §4.5 tests.
4. Follow-up: apply the §4.1 guard idea to `bundle:darwin` / Linux resolver.

## 8. Open questions

- O1. Cheapest reliable way to read the linked `cef` crate version in the task shell —
  `Cargo.lock` grep vs `cargo metadata`. (Lean toward `Cargo.lock`: no toolchain
  invocation, deterministic.)
- O2. Reading `libcef.dll` ProductVersion without PowerShell (for non-Windows CI cross-
  packaging)? A `strings`/PE-resource parse is possible but `pwsh` is already a Windows
  build dependency — acceptable.
- O3. Should the guard live in `build.rs` (fails the compile) instead of/in addition to
  `bundle:windows`? `bundle` is the integration point where both facts (linked crate +
  bundled dll) are known, so it's the natural home; a `build.rs` `rerun-if` on the cef
  version is a complementary belt-and-suspenders to force re-extraction.
