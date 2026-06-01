# Source Maps in Portable Builds

**Status:** Implemented  
**Date:** 2026-06-01

## Rule

- **`task package`** (dev portable) — maps **included**, channel = `dev-portable-<branch>`. Use for all local iteration and testing.
- **`task package:release`** (release portable) — maps **stripped**, channel = `stable` (baked via `RELEASE_CHANNEL=stable`). Use when building the artifact that ships in a GitHub release. Without this, the portable boots users into the dev-portable channel instead of their real data.
- **`task dev`** — maps always available (Vite dev server, no ZIP).

## Why

`vite.config.ts` sets `sourcemap: true` unconditionally so that the runtime
source-map resolver (`frontend/log/source-map-resolver.ts`) can rewrite
minified stack traces in the `task dev` terminal output.

The same build pipeline produces the portable ZIP. Without intervention,
~28 MB of `.map` files (7% of the ZIP) would ship in every portable — data
that serves no user-facing purpose at runtime.

Maps stripped in v0.41.1+. See `docs/analysis/ANALYSIS_PORTABLE_SIZE_REDUCTION_2026_06_01.md`
for the full size breakdown that motivated this.

## How it works

`scripts/package.sh` runs `find dist/frontend -name "*.map" -delete`
immediately after `task build:frontend` and before `task bundle`. The
frontend assets directory is already on disk at that point; the deletion is
instantaneous and does not affect the source files or the Vite output for
future builds (each `task build:frontend` regenerates `dist/frontend` from
scratch).

```
# task package:release (STRIP_MAPS=1):
task build:frontend        # generates dist/frontend/**  (maps included)
find dist/frontend \       # strip maps — release only
  -name "*.map" -delete
task build:backend
task build:host
task bundle                # ZIP from dist/frontend (no maps)

# task package (dev default, STRIP_MAPS unset):
task build:frontend        # maps included
# no deletion
task build:backend
task build:host
task bundle                # ZIP from dist/frontend (maps present)
```

## Source-map resolver behaviour without maps

`frontend/log/source-map-resolver.ts` resolves map files relative to the
frontend asset path at runtime. When no `.map` file is found it degrades
gracefully: the original minified frame is emitted as-is in the host log.
No crash, no missing log lines — the stack just shows transpiled names
(e.g. `e.x` instead of `blockId`) instead of the original TypeScript.

The resolver **is not disabled** in portables — it remains active so that
if a developer copies their own maps alongside a portable for a local debug
session, they work without any code change.

## What to do if maps are needed in a portable

Temporarily comment out the `find … -delete` line in `scripts/package.sh` for that build only. Do **not** commit that change.

## For future agents

- **Use `task package:release`** when building portables for GitHub releases.
  **Use `task package`** for all local dev/testing portables. Never swap these.
- If you add a new `.map`-generating build step (e.g. a WASM module), the
  `find dist/frontend -name "*.map" -delete` glob in `scripts/package.sh`
  covers it automatically — no extra wiring needed.
- The analysis document at
  `docs/analysis/ANALYSIS_PORTABLE_SIZE_REDUCTION_2026_06_01.md` tracks all
  portable size decisions — update it when making size-affecting changes.
- Do **not** set `STRIP_MAPS=1` outside of `task package:release` or CI.
  Dev portables with stripped maps break the source-map resolver and make
  local debugging harder.
