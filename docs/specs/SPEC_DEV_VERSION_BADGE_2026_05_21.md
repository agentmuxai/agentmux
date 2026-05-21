# SPEC — Build identification in the status bar

**Status:** Draft / for implementation
**Date:** 2026-05-21
**Author:** AgentA
**Area:** `frontend/app/statusbar/StatusBar.tsx` + `.scss`,
`frontend/app/statusbar/InstancePanel.tsx`, `frontend/types/custom.d.ts`,
`agentmux-cef/src/commands/platform.rs`, `agentmux-cef/build.rs`

Three small, related features. All make it possible to tell *which build* a
running window is — a recurring pain when several dev/portable builds run side
by side.

---

## Current state — there is no real build metadata

`get_about_modal_details` (`platform.rs:123-129`) returns:

```jsonc
{ "version": version, "buildTime": version, "platform": …, "arch": … }
```

`buildTime` is **hard-coded to the version string** — it is not a time. So the
Instance panel's "Build" row (`InstancePanel.tsx:294-298`, renders
`about().buildTime`) just re-displays the version already shown in the
"Version" row above it. There is **no commit hash and no build timestamp**
anywhere in the product. Features B and C add both.

Instance-panel rows today: **Version** → **Build** → **Runtime** (platform ·
arch).

---

## Feature A — DEV badge on the version chip  ✅ implemented

### Problem

A `task dev` build and a packaged portable show an identical version chip
(`vX.Y.Z`). With several builds running, there is no way to tell them apart —
this cost real time when a dev build and a portable were both `v0.37.6`.

### Behavior

When the running build is a dev build, the version chip shows a `DEV` badge to
the right of the version: `v0.37.6 DEV`. Portables are unchanged. The badge is
a non-interactive label.

### Implementation (done)

`StatusBar.tsx` — `<Show when={isDev()}><span class="status-version-dev">DEV
</span></Show>` inside the version button (and the offline-fallback span),
after `v{version}`. `isDev()` from `@/store/global`. `StatusBar.scss` —
`.status-version-dev` outlined accent tag.

---

## Feature B — "Build" row shows the commit hash

### Desired

The "Build" row shows the **truncated git commit hash** (`git rev-parse
--short HEAD`, ~7 chars) of the commit the build was made from — a precise,
non-redundant build identifier:

```
 Version   v0.37.6
 Build     a1b9f3c
```

### Implementation

1. **`agentmux-cef/build.rs`** — at compile time, run `git rev-parse --short
   HEAD`; emit `cargo:rustc-env=AGENTMUX_GIT_HASH=<hash>`. Fall back to
   `"unknown"` if git is unavailable / not a repo (source-tarball builds) —
   never fail the build. Emit `cargo:rerun-if-changed=.git/HEAD` (+ the active
   ref) so the hash refreshes when `HEAD` moves. *Optional:* suffix `-dirty`
   when `git status --porcelain` is non-empty.
2. **`platform.rs`** — add a `gitHash` field to `get_about_modal_details`,
   from `env!("AGENTMUX_GIT_HASH")`.
3. **`AboutModalDetails`** (`custom.d.ts`) — add `gitHash: string`.
4. **`InstancePanel.tsx`** — the "Build" row renders `about().gitHash`, mono.
   Keep/add a copy button (the "Version" row already has one).

---

## Feature C — new "Time" row with the build timestamp

### Desired

A new **"Time"** row, positioned **between "Build" and "Runtime"**, showing
when the build was produced:

```
 Version   v0.37.6
 Build     a1b9f3c
 Time      Jan 3, 2019 8:12AM
 Runtime   windows · x64
```

Format: `Jan 3, 2019 8:12AM` — abbreviated month, day with no leading zero,
full year, 12-hour clock with no leading zero, 2-digit minute, `AM`/`PM` with
**no space** before it.

### Implementation

1. **`agentmux-cef/build.rs`** — alongside the hash, emit the build time:
   `cargo:rustc-env=AGENTMUX_BUILD_TIME=<epoch-ms or RFC3339>`.
2. **`platform.rs`** — make `buildTime` a **real timestamp** at last (from
   `env!("AGENTMUX_BUILD_TIME")`), not the version string.
3. **`AboutModalDetails`** — `buildTime` stays, now genuinely a time
   (epoch-ms `number`, or an ISO `string` — pick one; epoch-ms is simplest to
   format).
4. **`InstancePanel.tsx`** — add the "Time" row between "Build" and "Runtime",
   rendering `about().buildTime` formatted as above.

### Formatting note

No date formatter exists in `frontend/util` today. `Intl.DateTimeFormat`
(`{month:"short", day:"numeric", year:"numeric", hour:"numeric",
minute:"2-digit", hour12:true}`) yields `Jan 3, 2019, 8:12 AM` — close, but
has a comma after the year and a space before `AM`. Strip both to match the
exact `Jan 3, 2019 8:12AM` target (small helper, co-located with InstancePanel
or a new `frontend/util` date helper).

---

## Open decisions

1. **`-dirty` suffix** on the hash — include now (recommended; a few lines in
   `build.rs`, most useful on the dev builds Feature A flags) or defer.
2. **`buildTime` type** — epoch-ms `number` vs ISO `string`. Spec recommends
   epoch-ms.
3. **Hash/time source for `agentmux-srv` / launcher** — out of scope; this spec
   embeds metadata in the CEF host only (`get_about_modal_details` runs there).

---

## Rollout

- **Feature A** — DEV badge. Trivial, frontend-only. **Implemented.**
- **Features B + C** — one PR: they share all the plumbing (`build.rs`
  embedding, `get_about_modal_details`, `AboutModalDetails`, the InstancePanel
  rows). Build-script + host + shared types.
