# SolidJS Migration Analysis — Performance & Bundle Size

**Date:** 2026-03-11  
**Current stack:** React 19 + Jotai + Vite + Tauri (WebView2/Chromium)

---

## Current bundle (production build)

| Chunk | Size (raw) | Size (gzip) | Notes |
|---|---|---|---|
| `wave-*.js` (main) | 2,169 KB | 654 KB | React + Jotai + xterm + all views |
| `mermaid-*.js` | 2,538 KB | 721 KB | Already lazy-split |
| `katex-*.js` | 261 KB | 77 KB | Already lazy-split |
| `wave-*.css` | 145 KB | 25 KB | Tailwind + SCSS |
| `index-*.js` | 36 KB | 10 KB | Entry bootstrap |
| **Total (main + entry)** | **~2,205 KB** | **~664 KB** | What loads on startup |

---

## Framework size comparison (gzipped)

| Framework | Gzip size | Notes |
|---|---|---|
| React 19 + ReactDOM | ~45 KB | JSX runtime included |
| Jotai | ~5 KB | Atom state library |
| **React total** | **~50 KB** | Current |
| SolidJS | ~7 KB | Signals built-in, no VDOM |
| **SolidJS total** | **~7 KB** | Replaces both React + Jotai |

**Framework savings: ~43 KB gzipped** — meaningful but not the dominant cost.

---

## Real bundle savings estimate

The main chunk (654 KB gz) contains:

| Contents | Estimated gz size |
|---|---|
| React + ReactDOM + Jotai | ~50 KB |
| xterm.js + addons | ~150 KB |
| All view components (TSX) | ~200 KB |
| Utilities, stores, types | ~150 KB |
| CSS-in-JS, misc | ~104 KB |

Replacing React + Jotai with SolidJS:
- **Save ~43 KB gzipped** on framework code
- **Save ~10-20 KB** from simpler reactive patterns (less boilerplate)
- **Total bundle reduction: ~55 KB gzipped (~8% of main chunk)**

Not a dramatic size win. The real gains are runtime performance, not bundle size.

---

## Runtime performance gains

This is where SolidJS genuinely wins for AgentMux's use cases:

### Terminal streaming (agent view, stream parsing)
- **React:** re-renders the component subtree on each token → `React.memo` and
  `useCallback` required everywhere to limit propagation
- **SolidJS:** only the reactive expression that reads the signal updates → zero
  VDOM diffing, direct DOM mutation
- **Estimated gain: 40-60% fewer DOM operations during streaming**

### Jotai atom reads (widget visibility, focus state, config)
- **React + Jotai:** atom change → re-render components that subscribed → VDOM diff
- **SolidJS signals:** signal change → only the JSX expression that reads the signal
  re-evaluates, nothing else
- **Estimated gain: ~3-5× faster fine-grained updates**

### Cold start / initial render
- Smaller JS parse/compile time (less code)
- No VDOM initialization overhead
- **Estimated gain: 20-30ms faster initial render** (Tauri loads from local disk,
  so network isn't the bottleneck — parse time is)

---

## What doesn't change

- **xterm.js** — framework-agnostic, unchanged
- **mermaid** — already lazy-loaded, unchanged
- **Tauri backend** — Rust, unchanged
- **CSS/SCSS** — unchanged

---

## Migration scope

| Category | Files | Effort |
|---|---|---|
| React component files (.tsx) | 89 files | High — JSX mostly ports 1:1 but prop destructuring breaks |
| Jotai atoms → SolidJS signals | 70 `useAtom` / `useAtomValue` sites | Medium — mechanical but numerous |
| Custom hooks → SolidJS primitives | ~20 hooks | Medium — logic same, API slightly different |
| `useEffect` → `createEffect` | ~40 sites | Low — near-identical |
| `useRef` | ~30 sites | Low-medium — lifecycle differs slightly |
| Context providers | ~5 | Low |
| **Total estimate** | | **2–4 weeks** careful work |

### Biggest gotcha
SolidJS does NOT work like React when props are destructured:
```tsx
// React: fine
function Foo({ value }) { return <div>{value}</div> }

// SolidJS: BREAKS reactivity — value captured at call time, never updates
function Foo({ value }) { return <div>{value}</div> }

// SolidJS: correct
function Foo(props) { return <div>{props.value}</div> }
```
Every destructured prop in the codebase is a potential silent bug. Requires
careful audit of all 89 component files.

---

## Alternative: Preact (lower risk, faster)

Preact is a React-compatible drop-in replacement (~3 KB gzipped vs 45 KB).

| Option | Bundle saving | Perf gain | Migration effort |
|---|---|---|---|
| **Preact** | ~42 KB gz | Low (still VDOM) | 1-3 days (compatibility shim) |
| **SolidJS** | ~55 KB gz | High (no VDOM) | 2-4 weeks |
| **Keep React** | 0 | 0 | 0 |

Preact via `vite.config`:
```ts
resolve: { alias: { react: 'preact/compat', 'react-dom': 'preact/compat' } }
```
Most of the codebase ports with zero changes. Jotai works with Preact.

---

## Quick wins (independent of framework)

These can be done NOW regardless of React vs SolidJS decision:

| Win | Estimated saving | Effort |
|---|---|---|
| Lazy-load agent view (heavy) | ~80 KB gz from main chunk | Low |
| Lazy-load markdown/mermaid renderer | Already done ✓ | — |
| Code-split per-view routes | ~50 KB gz from main chunk | Medium |
| `shiki` syntax highlighter lazy split | ~30 KB gz | Low (add to manualChunks) |
| Drop unused xterm addons | ~20 KB gz | Low |
| **Total quick wins** | **~180 KB gz (~27%)** | Low-Medium |

---

## Recommendation

**Phase 1 (this week):** Quick wins — lazy-split more chunks, drop unused
addons. Get main chunk from 654 KB → ~474 KB gzipped. Zero risk.

**Phase 2 (evaluate):** Profile the streaming render path. If agent message
streaming has measurable jank → SolidJS is justified for the agent view only
(hybrid approach: SolidJS for high-frequency views, React for chrome/layout).

**Phase 3 (if warranted):** Full SolidJS migration with careful prop audit.

The bundle size argument alone doesn't justify a full SolidJS rewrite —
~55 KB savings on a Tauri app loading from local disk is imperceptible.
The runtime performance argument (streaming, fine-grained updates) is stronger
but needs profiling data to confirm it's an actual bottleneck.

---

## SolidJS-specific packages available

| Need | Package |
|---|---|
| xterm.js binding | `solid-xterm` (community) or use xterm directly (works fine) |
| Routing | `@solidjs/router` |
| State (if needed beyond signals) | `@solidjs/store` (built-in) |
| Meta/head | `@solidjs/meta` |
| Testing | `solid-testing-library` |
