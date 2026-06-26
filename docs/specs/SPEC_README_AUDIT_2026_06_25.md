# README Audit — 2026-06-25

## Goal
Remove stale, internal-only, and verbose content from README.md. Target: a clean
public-facing document that a new user can read in <5 minutes.

## Issues identified

### Stale / wrong
| Line(s) | Problem | Fix |
|---------|---------|-----|
| 95 | `task package -- --fresh` listed as valid — it's a no-op (CLAUDE.md) | Delete |
| 101 | "task package:macos and task package:msix are TODO stubs" — internal caveat | Delete |
| 230 | Builder mention: "full release artifact set is produced by agentmuxai/agentmux-builder" — private repo, outdated | Replace with pointer to GitHub Releases |
| 286 | "Releases are built by agentmuxai/agentmux-builder" — obsolete | Remove entire builder subsection |
| 291–295 | "How it works" — describes old builder pipeline, not current workflows | Remove |
| 299–302 | "Triggering a release" with `gh workflow run tauri-build.yml -R agentmuxai/agentmux-builder` — dead | Remove |
| 304–312 | Release artifacts table: wrong filename patterns (aarch64 vs arm64), lists .deb we don't produce | Fix filenames, drop .deb |
| 316–343 | Full release checklist: steps 3–6 reference builder + landing site deploy, dead | Trim to steps 1–2 only |
| 277–280 | Badge `?job=macos` etc. — GitHub job-name filter requires exact name, but even so overall workflow is what fails | Simplify to single overall badge |

### Verbose / internal
| Section | Problem | Fix |
|---------|---------|-----|
| Lines 99–101 | Long explanation of local build labeling logic | Cut to 1 sentence, link CLAUDE.md |
| Lines 243–265 | Version management: `task release:patch`, `--dry-run`, `--as` warning, internal changesets note | Keep core commands only |
| Lines 337–343 | Steps 5–6 of checklist (landing site deploy) | Delete |

## Proposed structure after edit

```
[Early alpha warning]
[Logo + title + tagline + badges]
## The Problem
## What AgentMux Does
## Quick Start
  ### Prerequisites
  ### Development
  ### Production Build (trim)
  ### Logs
## Widgets
## Agents
## App API
## Architecture
## Build Commands
  ### Build Outputs (fix builder ref)
## Version Management (trim)
## Releases
  ### Download (nightly grid — fix badge)
  [remove builder subsections]
  ### Release artifacts (fix filenames)
  ### Release checklist (steps 1–2 only)
## Contact Us
## Disclaimer
## License
```

## Badge fix
Replace three per-job badges with one overall badge. The `?job=` query param only
matches on exact job `name:` field value (e.g. "macOS arm64 — sign + notarize DMG"),
not the YAML key. Since all jobs share one workflow, one badge is cleaner:

```markdown
[![Nightly builds](https://github.com/agentmuxai/agentmux/actions/workflows/ci-nightly-artifacts.yml/badge.svg)](https://github.com/agentmuxai/agentmux/actions/workflows/ci-nightly-artifacts.yml)
```
