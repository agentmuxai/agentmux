# Tauri & CEF Cleanup Audit

**Date:** 2026-04-17
**Audited by:** AgentA
**Status:** In progress (PR #417 covers partial cleanup)

---

## Summary

| Category | Count | Action |
|----------|-------|--------|
| Dead Tauri code (behind `isTauriHost()`) | 6 files | REMOVE in follow-up PR |
| `@tauri-apps/*` npm packages | 7 packages | Remove `@tauri-apps/cli`; others used as fallback |
| `src-tauri/` path references | 5 occurrences | REMOVE (directory doesn't exist) |
| Outdated docs (Tauri branding) | 3 files | UPDATE |
| CEF technical naming (`agentmux-cef`) | Many | KEEP — correct crate/binary name |

---

## Already Fixed (PR #417)

- CLAUDE.md: "CEF desktop application" → "Desktop application (Chromium-based)"
- CLAUDE.md: Removed "Tauri host has been removed" (no longer needed)
- CLAUDE.md: Removed stale Linux WebGL/WebKitGTK warning
- wave.ts: Removed dead multi-instance Tauri title code
- wave.ts: Deprecated `isTauriHost()` (returns false)
- Taskfile.yml: `cef:build` → `build:host`, `cef:package:portable` → `package`

---

## Remaining (follow-up PR)

### REMOVE: Dead Tauri imports inside unreachable code

| File | Lines | What |
|------|-------|------|
| `frontend/wave.ts` | 239, 260, 342, 359 | `import("@tauri-apps/api/window")` inside `if (isTauriHost())` blocks |
| `frontend/wave.ts` | 237-244, 258-263, 340-345, 357-362 | Entire `if (isTauriHost()) { ... }` blocks — unreachable |
| `frontend/tauri-bootstrap.ts` | 68, 105-106, 153 | `@tauri-apps/api/*` imports in dead bootstrap |
| `frontend/app/view/term/term.tsx` | 322 | `import("@tauri-apps/api/webview")` dead path |

### REMOVE: `@tauri-apps/cli` dependency

`package.json` lists `@tauri-apps/cli` v2.10.0 as a devDependency. This is the
Tauri build toolchain — no longer used since the Tauri host was removed. Safe to
uninstall: `npm uninstall @tauri-apps/cli`.

The other 6 `@tauri-apps/*` packages are referenced by the `ipc.ts` abstraction
layer as fallbacks. Once the dead code paths above are removed, these can be
evaluated for removal too.

### REMOVE: `src-tauri/` path references

| File | Lines | What |
|------|-------|------|
| `BUILD.md` | 118 | "Never launch from `src-tauri/target/` directly" |
| `scripts/benchmarks/measure-performance.sh` | 45-177 | 8x `src-tauri/target/release/` paths |
| `scripts/benchmarks/measure-performance.ps1` | 9, 132-133 | 3x `src-tauri\target\release\` paths |

### UPDATE: Outdated docs

| File | What |
|------|------|
| `BUILD.md:5` | "Built on Tauri v2" — should say Chromium-based |
| `CONTRIBUTING.md` | Tauri shell references |
| `README.md` | Mostly correct, minor Tauri mention |

### KEEP: CEF technical naming

The following are **correct technical identifiers** and should NOT be renamed:

- `agentmux-cef` — Rust crate name (the binary IS a CEF host)
- `agentmux-cef/src/` — source directory
- `cef-dll-sys` — build dependency
- `dist/cef-dev/` — dev mode output directory
- Comments in Rust code explaining CEF API usage
- `data/cef/` — CEF user data directory (Chromium profile)

These are internal implementation details, not user-facing branding. Renaming
the crate would be a breaking change with no user benefit.

---

## Recommended Follow-up

One PR that:
1. Deletes all `if (isTauriHost()) { ... }` blocks in `wave.ts`
2. Deletes `frontend/tauri-bootstrap.ts` (if still exists)
3. Removes `@tauri-apps/cli` from package.json
4. Cleans up `BUILD.md` and benchmark scripts
5. Evaluates remaining `@tauri-apps/*` packages for removal

Estimated effort: 1-2 hours.
