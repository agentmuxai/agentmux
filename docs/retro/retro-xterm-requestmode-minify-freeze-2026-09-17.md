# Retro: a minifier bug froze the terminal, but only in packaged builds

**Date:** 2026-09-17
**Author:** Opaz
**Status:** implemented — `safari13` dropped from the Vite target, esbuild floored
at >=0.28.2, and the terminal parser hardened against the failure class
**Severity:** High — any terminal pane running a program that queries DECRQM
becomes unresponsive; packaged builds only

---

## 1. Symptom

Running `vim` in a terminal pane froze the pane: no rendering, no input. It
reproduced reliably in packaged builds (0.56.3, 0.56.4) and **never** under
`task dev`. That split is what made it survive several investigations.

## 2. Root cause

`vite.config.ts` built with `target: ["es2021", "chrome97", "safari13"]`.
Safari 13 predates logical assignment, so esbuild lowers `a ||= {}`. On
xterm.js 6.0.0's `requestMode` it lowers it **wrong**.

Source (`@xterm/xterm/lib/xterm.mjs`):

```js
requestMode(e, i) {
  let r;
  ((P) => (P[P.NOT_RECOGNIZED = 0] = "NOT_RECOGNIZED", /* … */))(r ||= {});
```

Shipped, after minification:

```js
requestMode(t, n) {
  ((p) => (p[p.NOT_RECOGNIZED = 0] = "NOT_RECOGNIZED", /* … */))(void 0 || (r = {}));
```

esbuild constant-folds the read of `r` (always `undefined` from `let r;`) to
`void 0`, **drops the declaration as unused**, and **keeps the write**. ES
modules are always strict, so the orphaned assignment throws
`ReferenceError: r is not defined`.

Minimal repro, esbuild 0.28.1 (the version Vite 6.4.3 ships):

```js
export function requestMode(e, i) {
  let r;
  ((P) => (P[P.SET = 1] = "SET"))(r ||= {});
  return e + i;
}
```

```
$ npx esbuild repro.js --minify --format=esm --target=safari13
function N(r,E){return(e=>(e[e.SET=1]="SET"))(void 0||(t={})),r+E}…
$ node -e "import('./out.mjs').then(m => m.requestMode(1,2))"
ReferenceError: t is not defined
```

`es2021`, `chrome97` and `es2022` all emit correct output (`let t; … (t ||= {})`).
Only the Safari 13 target triggers it.

## 2a. This is a known upstream bug — we were one patch behind

It is [evanw/esbuild#4508](https://github.com/evanw/esbuild/issues/4508), fixed
in **esbuild 0.28.2**:

> Fix a minification bug with lowered logical assignment operators. This release
> fixes a bug that could cause esbuild to generate incorrect code for logical
> assignment operators when lowering them to an older target environment.

Upstream's explanation matches the diagnosis exactly: lowering duplicates the
left-hand side, esbuild failed to count the duplicate as a *new usage* when the
LHS is an identifier, so the minifier believed the variable was used once and
inlined its initializer into that use — dropping the declaration.

Vite 6.4.3 resolves `esbuild ^0.25.0`; we had **0.28.1**. Verified against both:

```
esbuild 0.28.1 --target=safari13  ->  void 0||(t={})      declaration dropped
esbuild 0.28.2 --target=safari13  ->  let E; ... E||(E={})  correct
```

So the two fixes are independent and either alone is sufficient for *this* bug:
removing the obsolete `safari13` target avoids the lowering path entirely, and
the `>=0.28.2` floor fixes the miscompile itself — which also covers any other
lowering path that hits the same flaw. Both are applied.

No upstream report is needed. The lesson is narrower and more uncomfortable: a
patch-level dependency gap produced a user-visible terminal freeze, and nothing
in our process would have surfaced it.

## 3. Why it froze the pane

`requestMode` is the **DECRQM** handler. Confirmed in the shipped bundle:

```js
this._parser.registerCsiHandler({ prefix: "?", intermediates: "$", final: "p" },
                                u => this.requestMode(u, !1))
```

So the throw happens inside xterm's escape-sequence parser. Captured live via
CDP:

```
requestMode  189:103840   ← throws
(anon)       189:78048
parse        189:68196
parse        189:83539
```

The exception aborts `parse` partway through the chunk, so the rest of that
buffer is never processed and the terminal is left mid-state — no rendering, no
input. A shell prompt never sends DECRQM; full-screen programs negotiating
terminal modes do, which is why it looked like "vim breaks the terminal".

## 4. Why it hid for so long

- **`task dev` never minifies.** Dev was always fine, which repeatedly pointed
  the investigation at the environment rather than the build.
- **The bug is not in the source.** Two independent source-level audits of the
  0.56.2→0.56.3 delta cleared it — correctly. Nothing in the diff is wrong; the
  *build* introduces the defect.
- **Source maps are stripped from packaged runtimes**
  (`scripts/stage-linux-runtime.sh` deletes `*.map`), so the renderer logged
  `Uncaught ReferenceError: r is not defined` at `index-….js (190)` with a 404
  for the map and no usable stack. `vite.config.ts` sets `sourcemap: true`
  specifically so `frontend/log/source-map-resolver.ts` can symbolicate at
  runtime — packaging then removes the maps that feature depends on. Worth its
  own fix; see §7.
- **Investigation tooling kept targeting the wrong instance.** Several rounds of
  testing were invalidated by two unrelated isolation defects found on the way:
  the AppImage extract-cache running an older binary (#3324) and pane
  environment inheritance (#3326). Neither caused the freeze; both made results
  untrustworthy.

## 5. Diagnosis that actually worked

Reading logs rather than driving anything:

| Instance | `is not defined` | behaviour |
|---|---|---|
| 0.56.2 | 0 (across 11,384 log lines) | vim works |
| 0.56.3 | 1 | freezes |
| 0.56.4 | 1 | freezes |

0.56.2 is *also* a minified AppImage, so minification alone was not sufficient —
which is what pointed at a build-configuration interaction rather than "packaged
builds are broken".

Then: symbolicate the minified position against the map left behind in
`dist/frontend/assets/*.map`, which resolved to `xterm.mjs`; read the shipped
code at that offset; reproduce in isolation with esbuild.

## 6. Fix

Drop `safari13` from the build target. It is a leftover from the Tauri/WebKitGTK
era — the frontend has rendered in CEF (bundled Chromium) on every platform
since, and every remaining WebKit reference in the tree is a historical comment.

Verified on a rebuilt bundle:

```
BEFORE: requestMode(t,n){(p=>(…)        ← no declaration
AFTER : requestMode(t,n){let r;(p=>(…)  ← restored
```

and the orphaned `void 0||(x={})` pattern no longer appears anywhere in the
output.

## 6a. Hardening the failure class

The specific miscompile is fixed twice over. The *class* is not: any uncaught
throw inside xterm's `parse()` — or a handler that hangs — wedges a pane the same
silent way, and that silence is why this took as long as it did.

`frontend/app/view/term/parser-wedge.ts` detects it by **liveness**, deliberately
not by inspecting `error.stack` for xterm frames: stack sniffing is unreliable
against minified vendor code, which is precisely the condition being survived.
`Terminal.write(data, cb)` invokes `cb` once that chunk is parsed, so writes
handed over and never acknowledged mean the parser stopped consuming — whatever
the cause.

On trip, `termwrap` resets the terminal (clearing half-finished parser state) and
calls `resyncController()`, the same path a crashed controller already takes. It
is a new trigger, not a new mechanism, and scrollback is re-read from the backend
rather than lost. Rate-limited so a repeatedly-failing pane is not thrashed, and
logged at error level — a pane that silently recovers still hides a bug.

Detection requires **both** unacknowledged writes and silence past the timeout.
Either alone is normal: an idle pane has no pending writes, a busy one settles.
7 tests pin the behaviour, including the partially-drained case that matches the
real failure shape (some chunks parse, then one throws and the rest stall).

## 7. Follow-ups

1. **Packaging strips the source maps the runtime resolver needs.**
   `stage-linux-runtime.sh` deletes `*.map` from the staged runtime while
   `vite.config.ts` emits them on purpose for `source-map-resolver.ts`. Either
   ship them or drop the resolver; today we pay the build cost and lose the
   benefit, and that directly cost this investigation.
2. **Dependency currency has no signal today.** The fix existed upstream in
   0.28.2 before we ever hit the bug; we shipped a terminal freeze because a
   transitive dev-dependency was one patch behind and nothing flagged it. Worth
   deciding whether build-toolchain versions deserve a currency check, given a
   minifier defect is invisible to every source-level review we run.
3. **Consider pinning the minifier target in one place with a comment**, so a
   future "add Safari support back" change cannot silently reintroduce this.
4. ~~An uncaught exception inside `parse` kills the terminal silently.~~ Done —
   see §6a.
