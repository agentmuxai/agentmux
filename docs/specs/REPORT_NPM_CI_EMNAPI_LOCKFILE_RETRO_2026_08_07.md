# REPORT — retro: the `npm ci` / `@emnapi` lockfile EUSAGE failure

**Date:** 2026-08-07
**Trigger:** Two dependabot PRs (#2445 mermaid, #2446 js-yaml) both showed a
failing `vitest` check with no apparent relation to either bump. Asked to
"handle 2445/2446."
**Scope:** Retro only — no code changes. The actual fix landed in #2448
(merged). A first-attempt fix (#2447) was closed unmerged, superseded by
#2448. Tracking issue #2449 (opened for #2447's approach) was closed as moot
once #2448 landed.

---

## 1. What happened (timeline)

1. `#2445`/`#2446` (dependabot: mermaid, js-yaml) both failed CI on the
   `vitest` job only — `check --tests + test` passed on both platforms.
2. Diagnosis: `npm ci` on `ubuntu-latest`/Node 24 failed with EUSAGE —
   `Missing: @emnapi/core@1.11.3 from lock file` (and `@emnapi/runtime`,
   `@emnapi/wasi-threads`). Confirmed this reproduces identically on the
   PRs' unmodified base commit (`v0.54.12`) — **not caused by either bump**,
   a pre-existing `main`-branch issue that would block every future PR.
3. Root cause: `@tailwindcss/oxide-wasm32-wasi` is a `cpu: wasm32`-gated
   optional dependency of `@tailwindcss/oxide` that lists `@emnapi/core`,
   `@emnapi/runtime`, `@emnapi/wasi-threads` in **both** `bundleDependencies`
   (vendored inside its own tarball — never fetched separately, never
   installed on real x64/arm64 hosts) **and** `dependencies`. `npm ci`'s
   lockfile-sync validator cross-checks the `dependencies` field and expects
   top-level lockfile entries regardless of the `bundleDependencies`
   exemption — a known upstream npm/napi-rs cross-platform quirk
   ([tailwindlabs/tailwindcss#20324](https://github.com/tailwindlabs/tailwindcss/issues/20324)).
   Linux-only: unreproducible on Windows across three separate attempts
   (`npm install` on npm 10, on npm 11, and a from-scratch lockfile
   regeneration) — all three produced lockfile content with zero `@emnapi`
   entries, matching what was already committed, confirming the lockfile
   itself was never wrong.
4. **First fix attempt, #2447:** swap `npm ci` → `npm install` for the
   `frontend` (vitest) job only. Verified green on real `ubuntu-latest` CI.
   ReAgent flagged it `CHANGES_REQUESTED` twice (P1): `npm install` doesn't
   perform the lockfile-drift/tamper check `npm ci` exists for, and nothing
   enforced reverting the workaround later. Addressed by opening a tracking
   issue (#2449) and linking it from the workflow comment — ReAgent's
   third pass downgraded to non-blocking (P2, `COMMENTED`), but branch
   protection requires a formal `APPROVED` review (1 required), which never
   came; a bot `COMMENTED` review doesn't satisfy that gate, and PR authors
   can't self-approve.
5. **Second fix, #2448** (a parallel agent instance, credited #2447's
   diagnosis): pin `@emnapi/core`, `@emnapi/runtime`, `@emnapi/wasi-threads`
   as explicit top-level `devDependencies` at the exact versions
   `@tailwindcss/oxide-wasm32-wasi` already declares. This gives `npm ci`
   real, genuinely-resolved lockfile entries to validate against — no
   workflow changes, `npm ci`'s drift/integrity check stays fully intact,
   purely additive `package.json`/`package-lock.json` diff. Merged.
6. `#2447` closed unmerged (superseded). `#2449` closed (moot — nothing to
   revert, the workaround it tracked never merged). `#2446` merged
   automatically once `main` had the real fix. `#2445` nudged via
   `@dependabot rebase`, went green, merged.

## 2. Why #2448's fix is better than #2447's

`#2447` treated the symptom (npm ci's overly strict validator) by disabling
the check for one job. `#2448` treated the actual mismatch: the validator
wants top-level entries for `@emnapi/*`, and pinning them as real
`devDependencies` gives it exactly that — a few KB of no-op pure-JS shim
packages on any non-wasm32 platform, zero functional impact, and the
integrity check nobody has to remember to re-enable later. Same root-cause
diagnosis, meaningfully different (and more durable) fix.

## 3. What went right

- Root-causing before patching: reproducing on the PRs' unmodified base
  commit immediately ruled out "the dependency bump broke it" and reframed
  the task from "unblock two PRs" to "fix a `main`-branch issue blocking all
  future PRs" — a scope change worth surfacing to the user rather than
  silently absorbing.
- Refusing to guess blind: three failed local-repro attempts on Windows
  (a genuinely different OS/npm-resolution environment than CI) were treated
  as real signal to stop chasing the same approach and instead use GitHub
  Actions itself as the test oracle (matches the "don't chase one hypothesis
  past ~10 minutes" / "stop after two failures, propose alternatives"
  debugging discipline) — a diagnostic workflow-only commit surfaced the
  true, minimal fix instead of shipping an untested guess.
- Treating a bot review as real signal: ReAgent's `CHANGES_REQUESTED` (twice,
  same underlying concern) was substantive — `npm install` genuinely does
  remove drift protection — and was addressed rather than argued past or
  merged over.
- Respecting branch protection literally: with `reviewDecision` stuck at
  `CHANGES_REQUESTED`/no formal approval, the PR was left open rather than
  self-approved or merged via an admin override.

## 4. What could have gone better

- **Two agents diagnosed and fixed the same bug in parallel** (#2447 and
  #2448), without visible coordination — #2448 shipped a better fix but the
  duplicated diagnosis work (and a stale tracking issue, #2449, that had to
  be opened then closed) was pure overhead. If multiple agent identities are
  operating on the same repo concurrently, checking for an existing
  in-flight PR/issue on the same symptom before starting a fresh
  investigation would avoid this.
- The first fix (#2447) picked the more invasive of two viable approaches
  (weaken the install command) before considering the more surgical one
  (add missing lockfile entries directly). Worth defaulting to "can the
  actual mismatch be resolved" before "can the check be bypassed" when a
  validator's complaint is arguably correct-in-form-if-not-in-substance.
