# Audit: "Command Line Inc." References in agentmuxai/agentmux

**Date:** 2026-03-26
**Repo:** agentmuxai/agentmux @ `0e938c7` (main)
**Goal:** Replace all "Command Line Inc." with "AgentMux Corp."

---

## Summary

| Category | Files | Pattern | Action |
|----------|-------|---------|--------|
| **LICENSE** | 1 | `Copyright 2025 Command Line Inc.` | Change to `Copyright 2025-2026 AgentMux Corp.` |
| **LEGAL.md** | 1 | Attribution in Third-Party Notices | Keep as-is (factual fork attribution) |
| **Rust backend** (`agentmuxsrv-rs/`) | 86 | `// Copyright 2025, Command Line Inc.` header (line 1) | `// Copyright 2025-2026, AgentMux Corp.` |
| **Rust wsh** (`wsh-rs/`) | 10 | Same copyright header | Same replacement |
| **Frontend** (`frontend/`) | 188 | Same copyright header | Same replacement |
| **BUILD.md** | 1 | Copyright header | Same replacement |
| **Docs/Specs** | 3 | Mention in analysis/specs | Update or leave as historical |
| **TOTAL** | **~290 files** | | |

---

## Category 1: Legal Documents (CRITICAL)

### LICENSE (line 189)
```
Copyright 2025 Command Line Inc.
```
**Action:** Change to `Copyright 2025-2026 AgentMux Corp.`

### LEGAL.md (line 29)
```
AgentMux is a fork of [Wave Terminal](https://github.com/wavetermdev/waveterm),
originally developed by Command Line Inc., licensed under the Apache License 2.0.
```
**Action:** Keep as-is. This is factual attribution of the upstream fork origin — legally required under Apache 2.0 Section 4(c). The rest of LEGAL.md already correctly references AgentMux Corp.

---

## Category 2: Source Code Copyright Headers (~284 files)

All source files (`.rs`, `.tsx`, `.ts`, `.scss`) have line-1 copyright headers like:
```
// Copyright 2025, Command Line Inc.
```

Some newer files (2026) have:
```
// Copyright 2026, Command Line Inc.
```

### Breakdown by directory

| Directory | Files | Year(s) |
|-----------|-------|---------|
| `agentmuxsrv-rs/src/backend/` | 86 | 2025, 2026 |
| `wsh-rs/src/` | 10 | 2025 |
| `frontend/app/` | 143 | 2024, 2025, 2026 |
| `frontend/layout/` | 23 | 2025 |
| `frontend/util/` | 14 | 2025 |
| `frontend/types/` | 4 | 2025 |
| `frontend/` (root files) | 4 | 2025 |
| `BUILD.md` | 1 | 2025 |

### Replacement plan

Use `sed` to do a single-pass replacement across all source files:

```bash
# Replace all copyright headers (preserving original year)
find agentmuxsrv-rs wsh-rs frontend BUILD.md -type f \
  \( -name "*.rs" -o -name "*.tsx" -o -name "*.ts" -o -name "*.scss" -o -name "*.css" -o -name "*.md" \) \
  -exec grep -l "Command Line Inc" {} \; | \
  xargs sed -i 's/Copyright \([0-9]\{4\}\), Command Line Inc\./Copyright \1-2026, AgentMux Corp./g'

# Fix LICENSE (different format, no comma)
sed -i 's/Copyright 2025 Command Line Inc\./Copyright 2025-2026 AgentMux Corp./' LICENSE
```

---

## Category 3: Docs with WaveTerm / Command Line Inc. Mentions

These files mention "Command Line Inc." or "Wave Terminal" in a historical/analytical context:

| File | Context | Action |
|------|---------|--------|
| `LEGAL.md:29` | Fork attribution | **Keep** (legally required) |
| `docs/analysis/dead-code-audit.md:86` | Notes that 327 files still need renaming | Update to reflect completion |
| `docs/specs/dead-code-strip.md:15` | "Inherited from WaveTerm (Command Line Inc, 2025)" | Keep as historical context |
| `specs/archive/rebrand.md` | Old rebrand spec | Keep as historical |
| `specs/CLEANUP_LEGACY_REMNANTS.md` | Cleanup tracking | Update to reflect completion |
| `README.md` | No Command Line Inc. references | Clean |

---

## Category 4: WaveTerm References (separate from copyright)

Files referencing "WaveTerm" or "waveterm" (not copyright headers):

| File | Context | Action |
|------|---------|--------|
| `LEGAL.md` | Fork attribution | Keep |
| `docs/analysis/dead-code-audit.md` | Audit notes | Keep as reference |
| `docs/specs/dead-code-strip.md` | Historical | Keep |
| `specs/archive/rebrand.md` | Old rebrand spec | Keep (archived) |
| `specs/archive/remove-aipanel-sidebar.md` | Historical | Keep |
| `specs/CLEANUP_LEGACY_REMNANTS.md` | Cleanup spec | Keep |
| `specs/LANDING_PAGE_INFRASTRUCTURE_SPEC.md` | Historical | Keep |
| `specs/readme-rewrite.md` | Historical | Keep |

No WaveTerm references found in active source code (`src-tauri/`, `agentmuxsrv-rs/`, `frontend/`, `wsh-rs/`).

---

## Execution Plan

1. Create branch `agent1/copyright-agentmux-corp`
2. Run sed replacement on ~284 source files (copyright headers)
3. Update LICENSE (line 189)
4. Update `docs/analysis/dead-code-audit.md` to reflect completion
5. Verify with `grep -r "Command Line Inc" --include="*.rs" --include="*.ts" ...`
6. Commit, PR, merge
7. Leave LEGAL.md fork attribution and archived specs untouched

## Risk
- **Low:** Copyright header changes are metadata-only, no logic changes
- **Legal note:** Keeping "Command Line Inc." in LEGAL.md fork attribution is correct — removing it would violate Apache 2.0 attribution requirements
