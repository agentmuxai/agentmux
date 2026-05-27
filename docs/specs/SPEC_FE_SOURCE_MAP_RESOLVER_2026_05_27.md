# SPEC: Frontend source-map resolver for piped error stacks

**Date:** 2026-05-27
**Status:** Draft — ready to implement
**Trigger:** Debugging the `replaceChild` crash in `AgentDocumentVirtualList` exposed that every captured `error.stack` in the host log shows raw minified positions (`bpe → wp → Object.fn → ...`). Resolving those positions today requires opening DevTools or hand-decoding `.map` files. Both are friction we shouldn't have for a tool we use to debug ourselves.

---

## 1. Why

Every uncaught error, unhandled promise rejection, and `console.error` call inside the renderer is already piped to the host log via `fe_log_structured` (see `frontend/log/error-forwarder.ts` + `frontend/log/log-pipe.ts`). The stack we forward is whatever `Error.stack` was at the moment of capture — which is the **minified bundle position** (line + column inside the rolled-up chunk). Chromium/V8 only apply source maps to stacks shown in DevTools; the runtime `error.stack` string stays raw.

To debug a renderer crash from a host log, we currently:

1. Find the chunk filename + line/column from the captured stack.
2. Open the bundled `.js` file, scroll to the column position, read minified code.
3. Open the matching `.js.map`, hand-decode the `mappings` field, or paste into a source-map web tool.

This costs 5-15 minutes per stack frame on a multi-frame crash. **The information IS in the bundle — there's a `.map` next to each `.js` file when Vite emits with `sourcemap: true`.** We just don't read it programmatically.

This spec adds a small runtime resolver that walks each captured `error.stack`, looks up each frame in the matching `.map`, and rewrites the stack to `original-file:original-line:original-column (originalName)` BEFORE forwarding to the host. Net effect: stacks in the host log are immediately useful — no DevTools dance, no hand-decoding.

The resolver is also a strict reliability win for the autonomous-agent workflow: when a Claude-driven agent debugs a renderer crash from logs, it gets source-mapped frames without needing a human at DevTools.

---

## 2. Design summary

A small async resolver, plugged into the error pipe at `error-forwarder.ts`. On the FIRST error of a given chunk filename it lazy-fetches the `.map`, parses it once, and caches the parsed consumer. Subsequent errors in that chunk are fully synchronous lookups.

Each error forwarded to the host:
- Carries the **raw** stack as `stack_raw` (current `stack` field, renamed) so we never lose ground truth.
- Carries a **resolved** stack as `stack` (new shape) — `${source}:${line}:${col} (${name})` per frame, where source/line/col come from the source map.
- Includes a `resolved: true | false | "partial"` flag so log consumers can tell what they're reading.

Async resolution is fire-and-forget; if the `.map` fetch isn't done yet, the resolved stack just shows the raw entries for those frames. The forwarder doesn't block on resolution — errors still pipe within the same microtask.

---

## 3. What gets resolved

`Error.stack` strings in V8/Chromium are formatted as:

```
NotFoundError: Failed to execute 'replaceChild' ...
    at bpe (http://127.0.0.1:54169/assets/index-BUzQ6xer.js:3:31187)
    at wp (http://127.0.0.1:54169/assets/index-BUzQ6xer.js:3:37060)
    at Object.fn (http://127.0.0.1:54169/assets/index-BUzQ6xer.js:3:36691)
    ...
```

A frame line matches the regex `^\s*at (?:(?<funcName>.+?) )?\((?<url>.+?):(?<line>\d+):(?<col>\d+)\)\s*$` (with a no-parens alternative for anonymous frames). The resolver:

1. Extracts `(funcName, url, line, col)` from each frame.
2. Maps `url` → `chunkName` (e.g. `index-BUzQ6xer.js`).
3. Loads `chunkName.map` if not cached (see §4).
4. Looks up `(line, col)` in the source-map consumer; gets `(source, origLine, origColumn, name)`.
5. Rewrites the frame as `    at ${name || funcName} (${source}:${origLine}:${origColumn})`.

Frames the resolver can't map (e.g. native frames, frames in chunks without a `.map`, partial-map gaps) pass through unchanged.

---

## 4. Caching + fetch strategy

**One-cache-per-chunk.** Parsed `SourceMapConsumer` objects are kept in a `Map<chunkName, SourceMapConsumer | Promise<SourceMapConsumer>>`. The map is module-scope in `frontend/log/source-map-resolver.ts`.

**Lazy load on first error in a chunk.** No upfront fetch. The first error pays a one-time `fetch(/*.map url*/)` + `new SourceMapConsumer(json)`. After that, lookups are synchronous.

**Storing the promise during fetch.** If two errors arrive while the same chunk's `.map` is in flight, they both attach to the same Promise — no double-fetch.

**Failure caching.** If the `.map` 404s or fails to parse, cache an explicit `Map.set(chunkName, FAILED_SENTINEL)` so subsequent errors in that chunk skip the network. Log once (to the host, via the same pipe but flagged) so we know maps are missing.

**Cache eviction:** none. The set of bundled chunks is bounded (~50-100 in our app); memory cost is modest (a few MB at most). No LRU needed.

**Pinned to chunk URL, not bundle hash.** Vite emits filenames like `index-BUzQ6xer.js` with a content hash. The cache key is the full filename, so a new build's chunks naturally get fresh cache entries.

---

## 5. Library choice

| Option | Pros | Cons |
|---|---|---|
| **`source-map-js`** (pure JS, ~50KB unpacked) | Maintained fork of Mozilla's `source-map`; sync API; no wasm | None significant |
| `source-map` (original) | Same API | Last release ages ago; uses wasm (extra fetch + complexity) |
| `stacktrace-js` | All-in-one wrapper | Heavier (~100KB); fetches maps itself but uses old source-map; opinionated frame format |
| Hand-rolled VLQ decoder | Zero deps | We're writing source-map software in 2026; no |

**Recommended: `source-map-js`.** Sync API, no wasm overhead, well-maintained, drops straight in.

---

## 6. Integration points

### 6.1 New module

`frontend/log/source-map-resolver.ts`:

```ts
import { SourceMapConsumer } from "source-map-js";

type CacheEntry = SourceMapConsumer | Promise<SourceMapConsumer> | "failed";
const cache = new Map<string, CacheEntry>();

/** Resolve a single stack frame line, async. Returns the original line if no map. */
async function resolveFrame(frame: string): Promise<string> { /* … */ }

/** Resolve every "at ..." line in a stack string. */
export async function resolveStack(stack: string): Promise<string> { /* … */ }

/** Sync fast-path: resolve frames whose maps are already cached; leave others raw. */
export function resolveStackSync(stack: string): { resolved: string; partial: boolean } { /* … */ }
```

Implementation notes:

- `resolveStackSync` returns whatever frames can be resolved RIGHT NOW from the cache, marking `partial: true` if any frame's map isn't cached. Used by the error forwarder so the host log gets SOMETHING immediately even if maps aren't ready.
- `resolveStack` is the async, complete version. Used by the forwarder to send a follow-up "resolved" log after the maps are fetched.

### 6.2 Wire into `error-forwarder.ts`

Today's `forward()` does:

```ts
invokeCommand("fe_log_structured", { level, module, message, data });
```

New shape:

```ts
async function forward(tag: string, fields: ErrorFields) {
    if (fields.stack) {
        const sync = resolveStackSync(fields.stack);
        const payload = {
            ...fields,
            stack_raw: fields.stack,                  // ground truth, always present
            stack: sync.resolved,                     // resolved-when-possible
            stack_resolved: sync.partial ? "partial" : true,
        };
        invokeCommand("fe_log_structured", { ..., data: payload });

        if (sync.partial) {
            // Maps weren't ready; resolve async + forward a follow-up.
            resolveStack(fields.stack).then((fullyResolved) => {
                invokeCommand("fe_log_structured", {
                    level: "WARN",
                    module: tag,
                    message: "(stack-resolved)",
                    data: { ...payload, stack: fullyResolved, stack_resolved: true },
                });
            });
        }
    } else {
        invokeCommand("fe_log_structured", { ..., data: fields });
    }
}
```

The synchronous-first emit means the host log gets the error at the same instant it does today (no added latency for the primary log line). The async follow-up arrives in a subsequent microtask once maps are loaded; it's tagged so log-readers can correlate the two entries by message+timestamp proximity.

### 6.3 Optionally extend `log-pipe.ts`

`log-pipe.ts` forwards every `console.*` call. Extending the same resolver to `console.error` calls that carry an `Error` argument is a small additional change — same shape, same `data.stack` rewrite. Out of scope for v1 — `console.error` paths often don't carry stacks anyway, and the uncaught-error / rejection paths catch the high-value cases.

---

## 7. Build pipeline considerations

### 7.1 Source maps in production builds

`vite.config.ts` currently emits source maps only when `NODE_ENV === "development"`. We have two choices:

**A. Always emit source maps in portable builds.** Pro: stacks are always resolvable from logs without a special debug bundle. Con: bundle size increases ~30 MB; possible IP leakage to anyone who unpacks the portable.

**B. Emit source maps only in debug builds.** Pro: no bundle bloat in normal portables. Con: when a user reports a crash from a normal portable, we still get raw stacks (resolver no-ops because maps aren't present).

**Recommended: A.** AgentMux is an internal-feel tool; source maps are not a meaningful IP risk. The 30 MB cost is negligible against a 165 MB portable. The "always-resolvable from any log" guarantee is worth the bytes. The `vite.config.ts` change is one line.

Out of scope here: if we later decide AgentMux ships to wider audiences, switch to a CDN-style "hidden maps" pattern (maps live at a different URL, not bundled in the portable). The resolver doesn't care WHERE it fetches from.

### 7.2 Map file accessibility from the renderer

CEF serves the portable's `runtime/frontend/` over `http://127.0.0.1:<port>/assets/...`. Source maps at `.map` URLs work automatically; no host-side change needed. (We verified this works — DevTools resolves stacks today when opened against our debug builds.)

---

## 8. Failure modes + handling

| Scenario | Behavior |
|---|---|
| `.map` file 404s | Cache `"failed"`. Log a one-time WARN "[source-map] missing for chunk X" via the same pipe (flagged so it doesn't recurse). All future frames in that chunk pass through raw. |
| `.map` parse fails | Same as 404 — `"failed"` sentinel, single WARN. |
| Frame parsing fails (unknown stack format) | Pass the line through unchanged. The full raw stack is also in `stack_raw` regardless. |
| Resolver itself throws | Catch at the top of `forward()`; emit the raw stack with `stack_resolved: "error"`. Never let logging fail. |
| Memory pressure | Cache size is bounded by chunk count (~50-100); no eviction. Each consumer is ~50-200KB. Worst-case ~20 MB. Acceptable. |
| Cross-origin chunk URLs | Same-origin only (the CEF dev server). Cross-origin maps would need CORS — out of scope. |

---

## 9. Performance contract

- **Primary error log line:** zero added latency. Synchronous resolver returns immediately with whatever's cached; emit the line; defer the async resolve to a follow-up.
- **First error in a chunk:** ~one-time 50-200ms (fetch + parse the map). Doesn't block the primary emit.
- **Subsequent errors in cached chunks:** ~1ms per frame for the lookup.
- **Bundle cost:** `source-map-js` is ~50KB unpacked → ~15KB gzipped. Negligible.

---

## 10. Testing

`frontend/log/source-map-resolver.test.ts`:

1. **Happy path.** Inline a synthetic `.map` JSON; verify `resolveStack(rawStack)` rewrites frames to `original.tsx:42:0 (functionName)`.
2. **Cache hit.** Two stacks in the same chunk; second call must not refetch.
3. **404 path.** Mock fetch to 404; verify `resolveStackSync` returns `partial: false` with raw stack, and the WARN is emitted once.
4. **Partial-map gap.** Frame whose `(line, col)` doesn't have a mapping; that one frame stays raw, others resolve.
5. **Non-Error stack format.** Frame lines that don't match the regex (e.g. native frames) pass through unchanged.
6. **Recursive-error safety.** The resolver itself throws inside resolveFrame; verify the forwarder still emits the raw stack and doesn't loop.

No integration test needed against the host log — that path is just `invokeCommand`, already covered by `error-forwarder.test.ts`.

---

## 11. Acceptance criteria

1. **A renderer crash logged to the host shows source-mapped frames.** Example: a deliberate `throw new Error("test")` in `agent-view.tsx` produces a host log entry whose `stack` field contains `at AgentPresentationView (frontend/app/view/agent/agent-view.tsx:204:8)` (or similar), NOT `at bpe (http://...:3:31187)`.
2. **The raw stack is still available** as `stack_raw` so we never lose ground truth.
3. **Primary log emission is synchronous.** Send 100 throws in a tight loop — no additional latency vs. today.
4. **Missing maps don't cause secondary errors.** Remove a `.map` from the portable; the same throw still emits the raw stack with a single "[source-map] missing" WARN.
5. **`task package` portable builds always include `.map` files** in `runtime/frontend/assets/`.
6. **Existing host-log consumers (muxlog, debug panels) still parse correctly.** The `data` payload change is additive (`stack_raw` is new; `stack` is now resolved when possible) — no field removal.

---

## 12. Out of scope (deferred)

- **Resolving `console.log/warn/info`** that happen to contain stack-like strings. We only touch the error / rejection paths; richer console wrapping is more invasive for less value.
- **Backend-side resolution.** Rust-side `agentmux_cef::commands::backend` just writes the payload to JSON. If we ever want to resolve stacks server-side (e.g. for a remote log forwarder), the parsing happens identically on a `.map` file — but the live debug case wants resolution AT THE SOURCE.
- **Stack-frame source-snippet inclusion.** Showing the actual source line in the resolved frame is doable (the `SourceMapConsumer.sourceContentFor(file)` API). Defer — log volume gets noisy.
- **Chrome `prepareStackTrace` hook.** V8 allows custom `Error.prepareStackTrace`, which would let us resolve at stack-construction time and have `error.stack` BE the resolved form globally. Defer — invasive (every error generation pays the cost) and trickier to test deterministically.
- **Symbolicating runtime evals (`new Function`, dynamic imports).** Out of scope; we don't use these in hot paths.

---

## 13. Files this redesign touches

```
package.json                                              + source-map-js dep
vite.config.ts                                            sourcemap: true (always)
frontend/log/source-map-resolver.ts                       NEW
frontend/log/source-map-resolver.test.ts                  NEW
frontend/log/error-forwarder.ts                           wire resolver into forward()
frontend/log/error-forwarder.test.ts                      add resolved-vs-raw assertion
agentmux-cef/src/commands/backend.rs                      (no change — just receives bigger payload)
```

Estimated diff: ~250 lines added (mostly the resolver + tests), ~10 lines modified.

---

## 14. Migration path

Single PR. No deprecations needed — the `data.stack` field is rewritten in place (raw is moved to `data.stack_raw`); log-consumers that currently parse `data.stack` get a more useful string for free.

For old logs already written with raw stacks, no migration. They stay raw; new ones get resolved.

---

## 15. Risk + revert

Risk: low. The resolver is a layer in front of an existing log pipe; if it fails, errors still get logged with raw stacks (just like today). The `try/catch` at the top of `forward()` enforces this.

Revert: drop the wire-in line in `error-forwarder.ts`. Resolver module + dep stays unused but harmless.

---

## 16. Open questions

1. **Should we also emit a one-line summary** before the resolved stack, e.g. `[stack-resolved 8/12 frames from index-BUzQ6xer.js]` so log readers can spot when resolution is partial? *Recommendation: yes, as a separate INFO log emitted alongside the first partial resolution per chunk. Cheap to add.*
2. **Should `error.cause` chains be resolved recursively?** When an error wraps another error (`new Error("…", { cause: inner })`), the inner stack also gets piped. *Recommendation: yes; the resolver should walk causes. Trivial extension once the core works.*
3. **Should we hash-version the cache keys** so multi-instance builds with different bundles in the same Chromium origin don't cross-pollute? The cache lives in renderer memory which is per-portable, so this doesn't apply today. *Recommendation: no, defer.*

---

*End of spec. Ready for go/no-go.*
