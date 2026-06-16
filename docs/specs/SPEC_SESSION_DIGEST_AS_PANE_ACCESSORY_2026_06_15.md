# SPEC: Session digest as a Pane Accessory

**Date:** 2026-06-15
**Status:** Draft — architecture extension (no code landed)
**Scope:** Agent pane session-digest surface (`frontend/app/view/agent/**`), the Pane Accessories model
**Extends:** `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md` §5 — **Pane Accessories** (Regions + the `<PaneRow>` primitive + "derive from a source of truth")
**Touches (today's digest):** `frontend/app/view/agent/hooks/useSessionDigest.ts`, `components/SessionDigestBanner.tsx`, `agentmux-srv/src/server/app_api.rs` (`session:digest`)

> This is a consumer spec: it does **not** redefine the accessory model — it adopts it. Read
> §5 of the forks/aux-pins spec first; this doc only says *how the session digest registers
> into that model*, and surfaces one small generalization the digest forces (single-row,
> meta-derived, transient accessories — §4).

---

## 1. Intent

The session digest (the AI one-line "Fixed auth bug, added dark mode, tests passing." banner)
is today a **bespoke surface**: its own component (`SessionDigestBanner`), its own hand-placed
slot in the `agent-view.tsx` flex stack, its own chrome. The Pane Accessories model (forks
spec §5) was created precisely to retire bespoke surfaces. **Bring the session digest under
that model — same regions, same row primitive, same derive-from-source discipline** — so it
stops being a one-off and becomes one more registered accessory.

The digest is already listed in the region map (forks spec §5.1, `top-fixed`: "progress ·
search · digest"). This doc makes that placement real and recasts the banner as a `<PaneRow>`.

---

## 2. Current state (what the digest is today)

Verified against `main` (commit context 2026-06-15):

- **Trigger** — `useSessionDigest.ts` on pane mount: shows a cached digest immediately; auto-
  generates in the background only when the session is **idle > 1h** *and* has **≥ 20 new
  lines** since the last digest (`IDLE_THRESHOLD_MS`, `STALE_LINE_THRESHOLD`). User can
  force-regenerate; user can dismiss (per-session).
- **Source of truth** — block meta: `session:digest_summary`, `session:digest_generated_at`,
  `session:digest_last_line_count`, gated on `session:line_count` / `session:last_activity_ms`
  (maintained by `blockcontroller/session_stats.rs`). The summary is itself **derived**: the
  `session:digest` RPC (`app_api.rs:1627`) reads the last ~200 transcript lines, prompts the
  pane's own CLI, and caches the result back into meta.
- **UI** — `SessionDigestBanner.tsx`: summary text + age + loading state + **regenerate** +
  **dismiss**, rendered above the conversation (`agent-view.tsx`).

This is *already* a clean derive-from-source surface (forks spec §5.3 rule 1 — satisfied). It
is also already **row-shaped**: an icon, a title (the summary), a meta (age), and two actions
(regenerate, dismiss), with an implicit status (cached / loading / stale / failed). So it is a
natural `<PaneRow>` — the only thing missing is that it doesn't *use* the shared primitive.

---

## 3. The mapping — session digest → Pane Accessory

### 3.1 Region

The digest registers into the **`top-fixed`** region (forks spec §5.1) — the transient-banner
slot at the very top of the pane, alongside the progress bar and search bar. Region contract
(`flex: 0 0 auto`, `flex-shrink: 0`, own `max-height`, declared z-order) is inherited; the
digest no longer chooses its pixels (forks §5.3 rule 2). When `<PaneRegions>` lands (forks
Phase 1), the digest is one entry in the region map, not a hand-ordered JSX line.

> Note: progress bar + search bar in `top-fixed` are **not** row-shaped and stay bespoke
> (forks §11). Only the digest converts to `<PaneRow>`.

### 3.2 The digest as a `<PaneRow>`

Project the digest's meta-backed state into the shared row interface (forks §5.2):

```ts
// A pure projection of block meta → PaneRow (no parallel store).
function digestRow(meta, state): PaneRow | null {
  if (state.dismissed) return null;          // per-session hide
  if (!state.summary && !state.loading) return null; // empty state → no row (zero cost)
  return {
    id: `digest:${blockId}`,
    sigil: "✦",                              // digest sigil (active/loading variant ↻)
    title: state.loading ? "Summarizing…" : state.summary,
    status: digestStatus(state, meta),       // see 3.3
    meta: state.generatedAt ? relativeAge(state.generatedAt) : undefined, // "2h ago"
    actions: [
      { id: "regenerate", glyph: "↻", run: () => digest.fetch(true) },
      { id: "dismiss",    glyph: "×", run: () => digest.dismiss() },
    ],
    expandable: false,                        // single line; no inline expand
    accent: statusColor(digestStatus(state, meta)),
  };
}
```

- **Single source of truth** (forks §5.3 rule 1): the row is a pure function of the digest
  meta + the hook's transient `loading`/`dismissed` signals. No new store. `useSessionDigest`
  keeps owning fetch/cache/dismiss; it just exposes a `row()` projection instead of feeding a
  bespoke banner.
- **Reuse `<PaneRow>` chrome** (rule 3): 3px status-accent left border, title ellipsis, action
  glyphs, interactive cursor — all from `_pane-row.scss`, identical to the dock/fork rows.

### 3.3 Status accent (reuse the dock palette)

Map digest lifecycle to the shared status colors (forks §5.2 — "running=green, error=red, …"):

| digest state | `status` | accent | when |
|---|---|---|---|
| generating | `running` | blue/green (pulse) | `loading === true` |
| fresh | `active` (or neutral) | accent | summary present, `linesSinceDigest < 20` |
| stale | `idle` | amber | summary present, `linesSinceDigest ≥ 20` (a refresh is warranted) |
| failed | `error` | red | last `fetch` errored / empty CLI result |

`linesSinceDigest = session:line_count − session:digest_last_line_count` — the exact staleness
signal the backend already computes (`app_api.rs:1670`). Surfacing it as an **amber stale
accent** is a UX win the bespoke banner doesn't have today: the row tells you *visually* when
the summary has drifted from the conversation, and one click on ↻ refreshes it.

### 3.4 Actions & conventions

- **Actions** map to `PaneRowAction`: `↻` regenerate (`fetch(true)`), `×` dismiss. Same action
  affordance as `stop ■` / `close ⌫` on other rows.
- **Empty state** (forks §7 discipline): no eligible digest → **no row** (zero cost for the
  common fresh-pane case), exactly like the dock with no processes / the fork bar with one
  fork.
- **Retention/overflow** (forks §5.2 D4/D6): N/A — the digest is a **single** row per pane,
  never a list. Dismiss is its retention. (This is the generalization §4 calls out.)
- **Cursor**: interactive row → `var(--cursor-interactive)`; the age meta is non-actionable →
  `var(--cursor-default)`.

---

## 4. What the digest adds to the model — single-row meta accessories

The forks spec's two exemplars (process dock, fork bar) are **multi-row, list-shaped**
accessories derived from a *collection* source of truth (ShellNodes; the instance set). The
digest is a **single-row, meta-derived, transient** accessory derived from a *scalar* source
(one cached summary in block meta). It fits `<PaneRow>` unchanged, but it clarifies the model:

- A region can host a **single `<PaneRow>`**, not only a list — `top-fixed` becomes the home
  for single-row meta accessories (digest now; future: "context window 82% full", "cost so
  far", "session age" — all meta-derived one-liners that should reuse this exact chrome).
- The list-only conventions (ordering D3, retention D4, overflow D6) are **opt-in per source**
  (forks §5.2 already says "a row source declares whether it opts in") — a single-row source
  opts out of all three and provides only dismiss.

Recommend adding one sentence to the forks spec §5.2 noting the single-row case so future
meta one-liners (context/cost/age) land on `<PaneRow>` in `top-fixed` by default rather than
re-inventing banners — the same mistake the digest made.

---

## 5. Phasing (rides the forks-spec phases)

| Phase | Deliverable | Depends on |
|---|---|---|
| **0 (now)** | This adoption spec; one-line addition to forks §5.2 for single-row sources. | — |
| **A** | When `<PaneRow>` + `_pane-row.scss` land (forks **Phase 1**), reskin `SessionDigestBanner` onto `<PaneRow>` via a `digestRow()` projection from `useSessionDigest`. **No behavior change** beyond the new stale-accent. | forks Phase 1 |
| **B** | When `<PaneRegions>` lands (forks **Phase 1**), register the digest into the `top-fixed` region map; delete the hand-placed JSX slot in `agent-view.tsx`. | forks Phase 1 |
| **C** | Add the **stale (amber)** accent + make `↻` prominent when stale; optional click-row-to-regenerate when stale. | A |

Phase A/B are pure refactors (pixel-identical except the new accent) gated on the forks
Phase-1 primitives existing — so this spec **does not** introduce `<PaneRow>`/`<PaneRegions>`;
it consumes them.

---

## 6. Non-goals

- Building `<PaneRow>` / `<PaneRegions>` — owned by the forks/aux-pins spec Phase 1.
- Changing how the digest is *generated* (the CLI summarization pipeline in `app_api.rs`) or
  its trigger thresholds — only the *surface* is recast.
- Making the progress bar / search bar into rows — they aren't row-shaped (forks §11).
- Multi-digest history / per-fork digests. (When forks land, each fork is its own block with
  its own digest meta — so a per-fork digest row is *free* later, but out of scope here.)

---

## 7. Why this is the right move

The digest is the cleanest possible candidate for the accessory model: it already derives from
a single source of truth, it's already row-shaped, and it already has exactly the actions and
status the `<PaneRow>` primitive exposes. Converting it (a) deletes bespoke chrome, (b) earns
the stale-accent UX for free, and (c) proves the model on a single-row case — establishing the
pattern so the next meta one-liner (context %, cost, age) is a 10-line projection, not another
hand-built banner. Same model, one more consumer.
