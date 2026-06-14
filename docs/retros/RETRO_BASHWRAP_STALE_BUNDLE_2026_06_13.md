# Retro: the exit-130 fix was correct for weeks — the agent never ran it

**Date:** 2026-06-13
**Severity:** High (masked a real fix; caused repeated "still broken" cycles)
**Status:** Root-caused. Immediate stopgap applied. Deterministic fix proposed (not yet shipped).

---

## TL;DR

The bashwrap **exit-130 fix (#1368) was correct and merged to `main`**, yet agents (e.g. Qooma) in a fresh `task dev` build kept hitting exit 130. Cause is **not** the code — it's the **build/bundle/resolution path**:

The agent's `agentmux-bashwrap` hook is a **bare PATH lookup**, the dev build **doesn't populate its own `tools/bin`**, the sidecar **appends** tool dirs ("system PATH wins"), and a **stale Downloads portable** (`agentmux-0.44.1+g6fc6d864.dirty.…-portable\runtime\tools\bin`, built *before* the fix) was sitting on the **system PATH**. So every agent resolved that stale binary instead of the build it was part of.

**The "fix didn't work" reports throughout the saga were a bundling artifact, not a code regression.** Manual tests passed because they ran `./target/release/agentmux-bashwrap.exe` directly; the *agent* ran a different, stale binary off the system PATH.

---

## How it actually resolves (the 3-layer trap)

1. **The hook is a bare command.** `agentmux-srv/src/backend/agent_config.rs:331` injects the Claude Code hook as `"command": "agentmux-bashwrap hook"` — resolved via the **agent process's PATH**, not an absolute path.

2. **Tool dirs are appended, gated on existence.** `agentmux-srv/src/backend/blockcontroller/shell.rs:~540`:
   ```rust
   // "Appended (not prepended) so system PATH always wins."
   // user store (~/.agentmux/tools/bin) — only if .exists()
   // bundled store (<exe_dir>/tools/bin) — only if .exists()
   c.env("PATH", format!("{current_path}{sep}{}", extra.join(sep)));
   ```
   - In a dev build the sidecar runs from `dist/cef-dev/runtime/`, so `bundled_tools_dir()` = `dist/cef-dev/runtime/tools/bin` — **which `task dev` never creates** → not added.
   - `~/.agentmux/tools/bin` — **doesn't exist** → not added.
   - Net: **no tool dir on the agent PATH**, and even if one existed it's appended → a system-PATH copy still wins.

3. **A stale portable was on the system PATH.**
   ```
   PATH ⊃ C:\Users\asafe\Downloads\agentmux-0.44.1+g6fc6d864.dirty.20260612T092359…-portable\runtime\tools\bin
   ```
   `g6fc6d864` predates #1368. `Get-Command agentmux-bashwrap` → that path. The agent ran it → exit 130.

### Evidence
`~/.agentmux/logs/bashwrap-debug.log`, current build, 08:12:38:
```
exec start tool_id=toolu_01Gq38… command_len=57
spawning bash -c via PTY …
PATH fix-up …
publisher done … chunks_published=0 chunks_failed=0     ← exit-130 signature
```
No `PTY child exited exit_code=` line — but that diagnostic line **is part of #1368**. Its absence proves the running binary predates the fix. (An earlier 01:57 entry that *did* run a fixed binary shows `PTY child exited exit_code=0`.)

`bundled_tools_dir()` also deliberately returns `None` under `target/debug|release` (so a bare `cargo run` host has no bundled store) — fine in principle, but it means dev correctness depends entirely on the assembled `dist/cef-dev/runtime/tools/bin`, which isn't assembled.

---

## Why the package path worked but dev didn't

Packaging copies the freshly-built tool into the portable: `cp target/release/agentmux-bashwrap.exe $PORTABLE/runtime/tools/bin/`. So **packaged** builds bundle the current binary. The **dev** path (`task dev` → `dev:serve` → `build:host` → `bundle`) assembles the CEF runtime but **never populates `runtime/tools/bin`**. Dev silently inherits whatever `agentmux-bashwrap` is first on the system PATH.

---

## Immediate stopgap (applied 2026-06-13)

Overwrote the PATH-winning stale binary with a fresh build:
```
cp -f target/release/agentmux-bashwrap.exe \
  "C:\Users\asafe\Downloads\agentmux-…g6fc6d864…-portable\runtime\tools\bin\agentmux-bashwrap.exe"
```
Verified the fresh binary: `echo …; cat; echo EOF_OK` → `<exited 0>` (no 130; the brace-group `{…} </dev/null` gives stdin-readers EOF without hanging). Qooma's next bash call uses it. **This is a hack — it patches a random Downloads build that happens to be on PATH. The build must stop depending on that.**

---

## Deterministic fix (proposed)

Two changes, both needed; **B is the real determinism fix**:

### A — Dev build bundles its own tools (`Taskfile.yml`)
Have the dev assembly (`dev:serve` / `build:host`) copy the freshly-built tool binaries into `dist/cef-dev/runtime/tools/bin/`, mirroring the package path. Then the bundled store exists for dev and contains the *current* binary. (Necessary but not sufficient — see B.)

### B — Resolve `agentmux-bashwrap` by absolute path, not bare PATH (the real fix)
`agentmux-bashwrap` is **the app's own version-locked streaming hook**, not a generic user tool like `jq`/`rg`. It must never be resolved by a bare PATH lookup that a stale system copy can win.
- Change the hook injection (`agent_config.rs`) to use the **absolute path to the bundled binary** (`<exe_dir>/tools/bin/agentmux-bashwrap[.exe]`, with the `target/release` dev fallback), e.g. `"command": "<abs>/agentmux-bashwrap hook"`.
- Equivalently, **prepend** AgentMux-owned tool dirs for app-owned binaries (keep append-semantics only for generic third-party tools). The current blanket "append so system PATH wins" is correct for `jq` but wrong for our own hook.
- This makes the binary version-locked to the running build and immune to any stale copy anywhere on PATH.

### C — Don't let stale portables poison the PATH (hygiene + guardrail)
- The stale `…Downloads\…portable\runtime\tools\bin` on the system PATH is the proximate trigger. Recommend removing portable `tools/bin` dirs from the persistent system/user PATH.
- Guardrail: at sidecar startup, **log the resolved `agentmux-bashwrap` path + its version** (add `--version` to bashwrap) and warn if it isn't the bundled one. A one-line "agent will use bashwrap at X (vY)" would have caught this in seconds.

### Verification plan for the fix
After A+B, on a fresh `task dev`: run an agent bash command and confirm `bashwrap-debug.log` shows the `PTY child exited exit_code=0` line (proves the #1368 binary) and that the resolved path is `dist/cef-dev/runtime/tools/bin/…`, regardless of what's on the system PATH.

---

## Lessons

1. **"Merged" ≠ "shipped to the runtime that runs it."** A fix landing on `main` and even in `target/release` doesn't mean the *agent* executes it. Always verify *which binary actually ran* (path + version), not just the source.
2. **Version-locked, app-owned binaries must be resolved by absolute path** — never a bare PATH lookup with app dirs appended behind the system PATH.
3. **Dev and package build paths must bundle identically.** Dev skipping `tools/bin` created a class of "works in package, broken in dev (or vice-versa)" bugs that are maddening to chase.
4. **Cheap observability would have saved the whole saga**: log the resolved hook binary path + version at agent spawn. Add `agentmux-bashwrap --version`.
5. The repeated "still broken" cycles cost real time precisely because we trusted the source/the merge instead of inspecting the live binary the agent invoked.

---

## Related
- #1368 (the exit-130 code fix — correct), D#1205 (floating-pane/tear-off thread), `docs/retros/RETRO_BASHWRAP_EXIT130_2026_06_12.md` (the original code-level retro — this one is the *delivery* retro).

*Written 2026-06-13 by AgentX.*
