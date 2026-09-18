# SPEC: Remove the `esbuild >=0.28.2` override — it broke `task dev` and never protected the bundle

**Date:** 2026-09-17
**Status:** Implemented
**Related:** #3343 (`87d86ef1`, the vim/DECRQM fix that added the override),
`docs/retro/retro-xterm-requestmode-minify-freeze-2026-09-17.md`,
evanw/esbuild#4508

---

## 1. Symptom

`task dev` came up as a blank window — AgentMux logo, no UI. The renderer never
got a working module graph because Vite's dependency optimizer failed before the
app was served, with 2,705 errors of this shape:

```
Transforming destructuring to the configured target environment
("chrome87", "edge88", "es2020", "firefox78", "safari14" + 2 overrides)
is not supported yet
```

Nothing in the frontend source changed to cause this. It started with
`87d86ef1` (#3343).

## 2. Root cause

#3343 added a top-level npm override:

```json
"overrides": { "esbuild": ">=0.28.2" }
```

An npm `overrides` entry is **global across the whole tree**, not scoped to the
package that motivated it. It forced `esbuild@0.28.2` into `vite@6.4.3`, which
declares `esbuild: "^0.25.0"` — a range 0.28.2 is outside of.

Vite's dep optimizer calls esbuild with its own target list plus two `supported`
overrides (`dynamic-import`, `import-meta` — `defaultEsbuildSupported` in
`vite/dist/node/chunks/dep-*.js`). Between 0.25.x and 0.28.x, esbuild changed how
an explicit `supported` map interacts with `target`: unlisted features are no
longer inferred from the target, so destructuring is treated as unsupported and
queued for lowering — and destructuring lowering is not implemented, hence the
error. Every dep containing a destructuring pattern fails; 2,705 is just the
count across our dep set.

Reproduced directly, same options Vite passes, one dependency:

```js
// node -e, esbuild.build({ target: ['es2020','edge88','firefox78','chrome87','safari14'],
//   supported: { 'dynamic-import': true, 'import-meta': true }, splitting: true,
//   bundle: true, format: 'esm', entryPoints: ['node_modules/solid-js/dist/solid.js'] })
esbuild 0.25.12 (vite's own)  -> OK - no error
esbuild 0.28.2  (forced)      -> ERRORS: 22
```

This is an ordinary version-range violation. npm does not warn about it: an
`overrides` entry is a deliberate instruction to ignore declared ranges.

## 3. The override never did what it was added to do

#3343's stated reason for the floor was that the DECRQM miscompile is
evanw/esbuild#4508, fixed upstream in 0.28.2, and that "Vite 6.4.3 resolves
esbuild ^0.25.0 and we had 0.28.1, one patch short."

The "we had 0.28.1" reading came from the hoisted root install. The tree before
#3343 was:

```
node_modules/esbuild              -> 0.28.1   (tsx@4.23.1, declares ~0.28.0)
node_modules/vite/node_modules/esbuild -> 0.25.12   (vite@6.4.3, declares ^0.25.0)
```

Vite had its own nested copy the whole time. **The 0.28.1 at the root belonged to
`tsx`, which does not touch the frontend bundle at all.** The shipped bundle was
built by 0.25.12, before and after #3343.

Which means the floor was a no-op in both directions:

- For `tsx`, `~0.28.0` already resolves to 0.28.2 (the newest 0.28.x) with or
  without the override.
- For Vite — the only esbuild that builds the bundle — it did not *raise* a
  version, it *forced an unsupported one*. That is the entire effect the
  override had on this repo.

And 0.25.12 still miscompiles, which the floor was supposed to prevent:

```js
// esbuild.transformSync(src, { minify: true, format: 'esm', target: 'safari13' })
// src: export function requestMode(e,i){ let r; ((P)=>(P[P.SET=1]="SET"))(r ||= {}); return e+i; }
0.25.12 -> function o(r,t){return(e=>e[e.SET=1]="SET")(void 0||(n={})),r+t}      <- declaration dropped
0.28.2  -> function o(r,n){let e;return(t=>t[t.SET=1]="SET")(e||(e={})),r+n}     <- correct
```

So the floor protected an esbuild that never built our bundle, while the esbuild
that did build it remained affected. The vim freeze was fixed anyway — by the
other half of #3343.

## 4. Why removing it is safe

The vim fix is the **target change**, not the version floor. #3343's own commit
message says so: "removing safari13 and this floor are independent fixes; either
alone is sufficient for this bug."

The miscompile only occurs when esbuild *lowers* `a ||= {}`, and it only lowers
when the target lacks logical assignment. `vite.config.ts` now builds with
`target: ["es2021", "chrome97"]` (`vite.config.ts:154`) — both support logical
assignment natively, so no lowering happens on any esbuild version:

```js
// esbuild 0.25.12 (vite's own, post-removal)
["es2021","chrome97"]            -> lowered: false | o.handlers ||= {};
["es2021","chrome97","safari13"] -> lowered: true  | o.handlers || (o.handlers = {});
```

`safari13` was a leftover from the Tauri/WebKitGTK era; the frontend has rendered
in CEF (bundled Chromium) on every platform since. It is gone, and there is no
target left in the build that triggers lowering.

The parser-wedge recovery added by #3343
(`frontend/app/view/term/parser-wedge.ts`) is untouched by this change and
remains the defense-in-depth layer for the *class* of bug.

## 5. Alternatives considered and rejected

**Upgrade Vite so 0.28.x is in range.** No version in the Vite 6/7 line accepts
0.28.x:

| Vite | declared esbuild range |
|---|---|
| 6.4.3 (current) | `^0.25.0` |
| 7.0 / 7.1 / 7.2 | `^0.25.0` |
| 7.3.0 | `^0.27.0` |
| 8.x | none — migrated to rolldown |

Vite 8 drops esbuild entirely for rolldown. That is a bundler migration, not a
dependency bump, and is not a fix for a broken dev server.

**Scope the override to Vite's own nested copy** (`"vite": { "esbuild": ... }`).
Tried; it hoists the pinned version to the root and invalidates `tsx`'s
`~0.28.0`. Trades one range violation for another.

**Keep the floor, pin Vite's copy separately.** Same problem, and it would be
maintaining a floor whose only real effect is the breakage — see §3.

## 6. Change

`package.json`:

```diff
 "overrides": {
-    "esbuild": ">=0.28.2",
     "brace-expansion@5.x": "^5.0.8"
 }
```

Resulting tree (the historical, correct layout):

```
tsx@4.23.1  -> esbuild@0.28.2
vite@6.4.3  -> esbuild@0.25.12
```

## 7. Verification

- `npm ls esbuild` shows the two-copy layout above, no forced version.
- `npx vite optimize --force` completes — previously 2,705 errors, now
  "Optimizing dependencies: @atlaskit/…, @codemirror/…, @xterm/…, solid-js, …"
  with none.
- `task dev` launches to a working UI rather than a bare logo.
- Lowering check (§4) shows `||=` is emitted verbatim under the shipped target on
  Vite's own esbuild, so the DECRQM handler cannot regress.
- `vim` in a terminal pane still works in a packaged build.

## 8. Follow-up

The real gap #3343 exposed is dependency currency, and that is unchanged here:
Vite 6.4.3 is pinned to an esbuild line that no longer receives the fix for
evanw/esbuild#4508. We are not exposed *today* because our target never triggers
lowering — but that is a property of one config line, not of the toolchain.
Moving off the 0.25.x line means moving Vite forward (7.3.0 for `^0.27.0`, or the
rolldown migration), which should be its own scoped piece of work rather than a
change smuggled in via `overrides`.

Worth recording as process, not just as a fix: `npm ls <pkg>` shows the hoisted
root copy first, and a nested copy is easy to miss. A version claim about what
built an artifact should be checked against the *nested* resolution for the tool
that actually built it.
