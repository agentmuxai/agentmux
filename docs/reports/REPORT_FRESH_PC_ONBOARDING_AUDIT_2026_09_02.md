# Report: fresh-PC onboarding audit — what exists, what's stale, what's missing

**Date:** 2026-09-02
**Trigger:** repo owner: "tighten up the toolchain and the process for new
users to get up and running on a brand new fresh PC with no tools installed —
the aim is for an install of agentmux to be a complete end-to-end development
solution." A fresh Windows install and a fresh Ubuntu install are available
for real end-to-end testing, not just a paper audit.
**Method:** full inventory of every onboarding-adjacent doc, script, task,
and CI workflow in the repo (via a dedicated Explore pass), then direct
verification of every claim against the current files on `main` at
`256648552` — no finding below is taken on the subagent's word alone.
**Status:** Investigation complete. No code changes yet. Tracking issue:
TBD (opened alongside this report — see bottom).

---

## 1. Is anyone already tracking this?

**No.** Searched open/closed issues and discussions for onboarding, bootstrap,
toolchain, "fresh install", "getting started", "new machine", "zero to dev",
"clean machine", prerequisites, installable — nothing tracks "fresh PC →
working `task dev`" as a project. The two nearest hits, #1130 and #1134, are
closed bug reports of `task dev` actually failing on a clean macOS checkout
(CEF/Rust binding drift) — useful evidence that fresh-clone fragility is a
real, recurring failure mode here, not a hypothetical one, but neither is a
tracking effort.

**One adjacent, already-shipped feature exists and must not be duplicated:**
`docs/specs/SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md` — a one-click
in-app installer (git/Node/npm/Python via winget/brew/pkexec) that fires from
`AgentPrereqModal` when a *launched agent's own CLI* (Claude Code, Copilot,
etc.) needs a missing tool. Implemented and tested for git/Node/npm/Python
across all three OSes. **This solves a different problem** — it's the shipped
app installing tools for its own end users at agent-launch time, not getting
the *dev checkout itself* building. The design patterns are directly
reusable (fixed command catalog, no shell interpolation, consent step,
winget/brew/pkexec per-OS strategy) — this audit's plan should cross-reference
it rather than reinvent the detection/install logic, but should not be
conflated with it in scope or in the tracking issue.

Two explicitly unimplemented pieces of that spec are directly relevant here
too: bootstrapping a *missing package manager itself* (no Homebrew-on-a-bare-Mac,
no winget-bootstrap) is flagged `[DECISION NEEDED]` and not built, and the
Windows UAC-prompt-origin behavior is flagged as **not yet verified on real
hardware**. Both gaps sit exactly on the path of "brand new PC, nothing
installed" — the fresh Windows/Ubuntu machines available for this project are
a chance to close the verification gap on the second one even if it's not
this project's code to fix.

---

## 2. Confirmed, concrete defects (verified directly, not inferred)

### 2a. Node version: README.md and BUILD.md both say 22, everything else says 24

```
.nvmrc                          -> 24
package.json "engines"          -> {"node": ">=24", "npm": ">=11"}
package.json "packageManager"   -> "npm@11.6.2"
every CI workflow (7 files)     -> actions/setup-node@v4, node-version: 24 / '24'
README.md line 73                -> | **Node.js** | 22 LTS | Frontend build |
BUILD.md line 15                 -> | **Node.js** | v22 LTS | Frontend build (SolidJS/Vite) |
```

A brand-new user following either root doc literally installs Node 22, then
hits `npm install` against an `engines` field requiring ≥24. This is the
single cheapest, highest-value fix available — flatly wrong today, no
judgment call needed.

### 2b. `task init` exists, is undocumented, and does almost nothing

`Taskfile.yml` line 371:

```yaml
init:
    desc: Initialize the project for development.
    cmds:
        - npm install
```

`desc:` promises "initialize the project for development" but the task is
`npm install` and nothing else — no Rust check, no CMake/Ninja check, no Task
self-check, no CEF prefetch. It is not referenced by name in README.md,
BUILD.md, or CONTRIBUTING.md — a working entrypoint whose name matches
exactly what this project needs, sitting unused and undersized. This is the
natural place to grow a real bootstrap task rather than invent a fourth,
competing entrypoint.

### 2c. BUILD.md's Linux apt list is missing packages CI actually installs

```
BUILD.md's documented list:  cmake ninja-build build-essential curl wget file libssl-dev git zip
CI's actual list (build-linux.yml): ninja-build cmake libwayland-dev libxkbcommon-dev libgtk-3-dev
```

`libwayland-dev`, `libxkbcommon-dev`, `libgtk-3-dev` are installed by CI but
absent from BUILD.md. A from-scratch Linux build following only BUILD.md's
list risks a missing-library link error CI never hits. This is exactly the
kind of gap the fresh-Ubuntu test machine can either confirm or rule out —
CI runs in a container image that may have some of these preinstalled;
whether they're genuinely required on a bare Ubuntu desktop install is an
open question this project should answer empirically rather than by reading
the CI YAML.

### 2d. Stale PATH hint in `Taskfile.yml`

The `VERSION` var's shell fallback references
`/opt/homebrew/opt/node@20/bin` — a Homebrew Node 20 formula path,
inconsistent with the real `>=24` requirement. Low-severity (it's a fallback
PATH entry, not a hard requirement), but worth correcting alongside 2a so
"20" doesn't linger anywhere near "which Node do I need."

---

## 3. What already exists and should be reused, not rebuilt

| Asset | Covers | Reuse how |
|---|---|---|
| `BUILD.md` | Fullest of the three root docs — real per-OS install commands, troubleshooting section, quick-reference table | Base to correct and extend, not replace |
| `README.md` Prerequisites table | Quick-glance version/purpose table | Correct in lockstep with BUILD.md so they can't re-diverge |
| `docs/linux.md` | AppArmor userns-sandbox blocker (Ubuntu 22.04+/23.10+ backported restriction breaks all CEF/Electron/Chromium apps), install/extraction, single-instance sockets | Fold the AppArmor gotcha into the fresh-Ubuntu path explicitly — this is a real first-run blocker the Ubuntu test machine will almost certainly hit |
| `scripts/install-userns-apparmor-fix.sh` | Installs the narrow AppArmor exception, today invoked via `pkexec` from the app's own "Fix it now" dialog | Confirm it also works run manually pre-install (docs/linux.md documents a manual alternative) — verify on the real fresh-Ubuntu box |
| `SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md` | Detect/install pattern (winget/brew/pkexec, fixed catalog, consent step) | Reference/reuse the *pattern*, not the code path — that installer targets already-launched-app agent prereqs, this project targets the dev checkout before the app is even built |
| CI workflows (`build-{windows,linux,macos}.yml`) | Currently the most authoritative, tested source of true per-OS install commands | Treat as ground truth when BUILD.md and CI disagree (as in 2c) |

## 4. What's genuinely missing (nothing to consolidate — greenfield)

- **No script or task anywhere checks for Rust/CMake/Ninja/Task/git presence.**
  `task dev` assumes the full toolchain is already correct; a missing CMake
  or Ninja surfaces as a raw `cargo`/`cef-dll-sys` build error, not a
  friendly "install CMake and Ninja" message.
- **No "fresh clone → working `task dev`" smoke test in CI.** CI builds from
  an already-checked-out branch in a maintained runner image; it doesn't
  exercise what a brand-new contributor's first run actually looks like on a
  machine with nothing preinstalled. #1130/#1134 (closed) show this gap has
  bitten before, on macOS specifically.
- **No single doc walks Windows/macOS/Linux from bare OS to `task dev`
  succeeding.** BUILD.md is close but incomplete (2c) and version-stale (2a).

## 5. Explicitly out of scope

**The patched-libcef-from-source build path** (`docs/cef-build/*.md`,
`scripts/cef-build/*`) — depot_tools, `gclient sync`, tens of GB, multi-hour
build. This is only needed for CEF-patch iteration or release artifacts with
full window-drag parity, not ordinary development. Ordinary `task dev` fetches
a prebuilt CEF automatically via `cef-dll-sys`'s own build script (cached
under `target/*/build/cef-dll-sys-*/out/`) — CMake+Ninja are only building
CEF's small C++ wrapper shim, not Chromium itself. A fresh-PC bootstrap
should guarantee CMake+Ninja+platform-C++-toolchain are present and stop
there; scope creep into the from-source path would turn a "get building in
20 minutes" goal into a multi-hour one for no reason most contributors need.

---

## 6. Proposed plan

1. **Fix the confirmed defects (2a–2d)** — cheap, unambiguous, no design
   decisions required. Do this first regardless of what else follows.
2. **Grow `task init` into a real bootstrap entrypoint**: detect Rust/Node/Task/CMake/Ninja/git
   presence and version, print clear per-OS install instructions for
   anything missing (reusing BUILD.md's corrected command list, and the
   toolchain-installer spec's detection *pattern* where it fits), then run
   `npm install`. Document it by name in README.md and BUILD.md so it stops
   being an undiscoverable no-op.
3. **Reconcile BUILD.md's Linux prerequisite list against CI's**, verified
   empirically on the real fresh-Ubuntu machine rather than assumed from the
   YAML diff alone (2c is a hypothesis until tested).
4. **Fold the AppArmor userns-sandbox gotcha into the Linux onboarding path**
   explicitly, with a tested pre-install manual step, verified on the real
   fresh-Ubuntu machine.
5. **Real end-to-end test on both provided machines**: bare Windows install
   and bare Ubuntu install, following only the (corrected) docs, timed and
   logged step by step, every deviation from the documented path recorded as
   a doc or tooling gap — not silently worked around.
6. **Add a fresh-clone CI smoke test** if the manual runs above justify it —
   a job that starts from a container/image with nothing preinstalled and
   runs only documented steps through to `task dev` succeeding, so #1130/#1134-class
   regressions are caught automatically instead of by a contributor's bug report.

---

## 7. Open questions for the tracking issue

- Should the bootstrap step (item 2) *offer* to install missing tools itself
  (mirroring the toolchain-installer spec's consent-based winget/brew/pkexec
  pattern), or only detect-and-print instructions? Auto-install is more
  "complete end-to-end solution" but is a bigger, riskier scope addition and
  duplicates more of the existing installer's territory.
- Is a from-nothing package-manager bootstrap (no winget/brew present at all)
  in scope for the *dev* checkout, given it's explicitly out of scope for the
  toolchain-installer spec's shipped-app version? Fresh Windows ships winget
  by default (recent builds); fresh Ubuntu ships apt by default — likely a
  non-issue in practice, worth confirming on the real machines rather than
  assuming.
