# Pillar 1 — Host Reproject: Open-Question Resolutions & Design Foundation

**Date:** 2026-06-30
**Type:** Design resolution (answers the four open questions gating Pillar 1)
**Status:** Conclusions agreed — ready to turn into a sized implementation spec
**Owner:** asaf
**Resolves:** Q1–Q4 in `docs/architecture/DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md` §7
**Builds on:** `SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md` (Pillar 1), Pillar 2/3 merged work

> Purpose: close the four open questions that gate Pillar 1 (host becomes a disposable
> projection of srv), grounded in (a) the actual host state inventory and (b) the
> recovery-architecture literature, so we can write a sized implementation spec next.

---

## 0. TL;DR

Pillar 1 is, in the literature's terms, making the host a **crash-only, microrebootable,
soft-state component** (Candea & Fox). The host already *is* a projection on cold start — it
rebuilds every window from srv. Pillar 1 = make that startup-restore path fire **unplanned,
mid-session**, and make it cover **all** host topology, not most.

Four resolutions:
- **Q1 (what moves to srv):** Only the **logical topology** (Category A) — window set + kind +
  parent linkage, layout tree, per-window opacity, floating-pane placement/restore-rects, and the
  pool *target size* (not pool contents). Native handles (Category B) and in-flight queues
  (Category C) are **rebuilt or dropped**, never serialized. Most of the topology is *already*
  mirrored in srv/launcher; the gap is small and enumerable (below).
- **Q2 (UX bar):** Reproject is a **visible, deliberate, one-shot rebuild** — target < ~1.5 s to
  first painted topology, covered by a branded "Restoring session…" overlay, never a raw OS fault
  box. Flicker is crash-path only. We **do not** chase invisible/instant recovery (that's the trap
  that produced 6× renderer-OOM re-fixes).
- **Q3 (write-through mechanism):** **State-store snapshot write-through**, not an event-sourced
  log. The host writes its current logical topology to srv's existing `db_layout`/window stores
  through **one path** (the reducer), async/batched, last-writer-wins per key. This deliberately
  ties into **#864** (retire the wcore-direct second write path). Event-sourcing is rejected as
  over-engineering for soft state that has no audit/replay requirement.
- **Q4 (admission control independence):** **Yes — already proven.** Pillar 3 shipped (#1853)
  independently of the host rework. Confirmed answer: admission control neither needs nor waits on
  Pillar 1.

---

## 1. Framing — what the literature says this actually is

The refactor direction maps cleanly onto three well-studied ideas. We are not inventing; we are
applying a known pattern that the codebase half-implements already.

- **Crash-only software** (Candea & Fox, HotOS-IX 2003): a component should have *one* way to stop
  (crash) and *one* way to start (recover). Separate shutdown/startup code is where bugs breed.
  **Prerequisite:** program state lives in dedicated **state stores**, not inside the crashable
  component. → Our host's "graceful flush vs. crash" incoherence (audit §2.2) is exactly the
  separate-shutdown-path smell; the fix is to make srv the state store so the host has *only* the
  crash/recover path.
- **Microreboot / Recovery-Oriented Computing** (Candea et al., OSDI 2004): restart fine-grained
  components instead of the whole system; requires **separation of logic execution from state
  management, loose coupling, soft state, and lease-based resources.** Demonstrated >95% reduction
  in failure *duration*. → A host reproject is a microreboot of the single most failure-prone
  component. The host's data must be **soft state** (reconstructable, leased), which the inventory
  below confirms it nearly is.
- **Event-sourcing snapshotting** (industry practice): replaying a long event log to rebuild state
  is slow; snapshots bound it; and write-through should be **async/background so the client never
  eats the persistence latency.** → Informs Q3: we want the snapshot half (current-state
  write-through), not the log half (we have no replay/audit need).

The throughline: **soft state in a dedicated store + one recover path = cheap, safe restart.**
That is precisely the three-tier model already decided.

---

## 2. Q1 — The host's authoritative state set (what must move to srv)

Full field inventory from `agentmux-cef/src/state.rs` + `reducer/`, grouped by disposition.

### 2.A — LOGICAL TOPOLOGY → **must be authoritative in srv** (the actual Pillar 1 work)
| Host field | What it is | In srv today? | Action |
|---|---|---|---|
| window set (via `shadow_window_meta` / `window_meta`) | which top-level windows exist, kind, parent linkage | **Yes** — launcher owns `instance_registry`, `backend_window_ids`, `windows`; host already projects them | Confirm completeness; close any host-only window facts |
| layout tree (pane topology) | the split/tab structure per window | **Partially** — `db_layout` exists but is written via **two paths** (reducer + wcore-direct) | **#864**: collapse to one write path; this is the core gap |
| `window_opacities: HashMap<String,f32>` | per-window opacity | **No** | Persist to window metadata |
| `pane_window_states: HashMap<String,PaneWindowState>` | floating-pane Normal/Max/Min + `last_known_normal_rect` | **No** | Persist restore-rects + placement |
| pool **target size** | how many warm windows/panes to keep | config | Keep in config (already durable) |

### 2.B — NATIVE / EPHEMERAL HANDLES → **rebuilt on reproject, never serialized**
`browsers: HashMap<String,BrowserHandle>` (CEF `Browser` FFI objects), GPU/renderer processes,
`pool.queue` / `pool.unpromoted` / `pane_pool.*` (warm-window live handles), `active_drag:
Option<DragSession>`. Only the *label/kind/metadata* of a browser has logical meaning; the handle
itself is recreated by replaying topology through `post_create_window`. **Pools rebuild empty** —
correct and acceptable (a fresh host warms them from target size). **Drag cannot outlive a restart
— dropped by definition.**

### 2.C — TRANSIENT / IN-FLIGHT → **dropped on reproject** (must be *resumable from topology*, not preserved)
`pending_window_creations`, `pending_browser_pane_creates`, `browser_panes` Live/Closing lifecycle,
`pool.respawn_in_flight` / `pane_pool.respawn_in_flight`, `quit_state` (Draining), dormant
`top_level_creation`. These are mid-flight operations. **Design rule:** the reproject must derive
the *desired* end state from srv topology and re-drive creation idempotently — it must **not** try
to resume a half-finished create. (This is the soft-state requirement: in-flight work is
reconstructed from durable intent, not checkpointed.)

### 2.D — already-projected metadata (no work)
`shadow_instance_registry`, `shadow_backend_window_ids`, `shadow_window_meta` are already read-only
projections of launcher truth. They *validate the model* — the asymmetry is that topology in 2.A
isn't yet treated the same way.

**Q1 answer:** the authoritative set that must move is **small and mostly already there** — it is
{layout tree via one write path (#864), per-window opacity, floating-pane placement/restore-rects}.
Everything else is either already in srv (2.D), a rebuildable native handle (2.B), or in-flight
work that must be re-derived rather than preserved (2.C). This is why Pillar 1 is "deep but not
huge": the hard part is *discipline* (one write path, no host-only truth), not volume.

---

## 3. Q2 — Reproject UX bar

**Decision: reproject is an honest, bounded, covered rebuild — not invisible recovery.**

- **Latency target:** < ~1.5 s from host (re)launch to first-painted topology (windows + layout in
  place), pools warming in the background after. Justification: cold start already does this; we're
  reusing that path, not adding a slower one.
- **Visual treatment:** a single branded **"Restoring session…"** overlay on reproject, then the
  topology paints in. **Never** the raw `0xE0000008` OS fault box (that's the
  `WER_FAULT_REPORTING_NO_UI` + native-dialog work already specced in `SPEC_GRACEFUL_OOM_EXIT`).
- **Explicitly NOT a goal:** zero-flicker / seamless in-place recovery. The audit shows chasing
  invisibility is what produced the 6×-refixed renderer-OOM saga. Per crash-only doctrine, a clean
  visible restart beats a clever invisible one. **Flicker is a crash-path event only** — steady
  state never rebuilds, so users see the overlay only when something actually died.
- **Acceptance:** the E2E test "host OOM ⇒ session reprojects" asserts topology equivalence
  (same windows/panes/layout) within the latency budget, with the overlay shown and no orphaned
  Job-Object tree (ties to Pillar 2 Stage 3's exit test).

---

## 4. Q3 — Write-through mechanism

**Decision: snapshot-style state-store write-through through the reducer — NOT event sourcing.**

Why not event-sourced log:
- Host topology is **soft state with no audit, time-travel, or replay-from-zero requirement.** An
  event log's payoff (full history, temporal queries, rebuild-from-events) is value we'd never use.
- The literature's own caveat: a growing event log *adds* recovery latency and forces you to bolt
  on snapshots anyway. We'd be paying the log's complexity to immediately neutralize it. Skip to
  the snapshot.

What we do instead:
- The reducer is the **single writer.** Every topology-mutating `HostCommand` (window open/close,
  layout change, opacity, floating placement) write-throughs the affected keys to srv's existing
  stores (`db_layout`, window metadata) **last-writer-wins per key**, async + batched so it never
  stalls the UI thread (the "background write-through" pattern — client never eats persistence
  latency).
- This is **not a new store** — it extends the path srv already uses for layout. The required move
  is **#864: retire the wcore-direct second write path** so the reducer is the *only* writer.
  Pillar 1's durability correctness is impossible while two writers race one record (audit §2.3
  "layout split-brain"); fixing #864 is therefore a **hard prerequisite**, not a nice-to-have.
- **Reproject read path = the existing cold-start restore.** No new deserialization code; the host
  already reads srv topology on launch. Pillar 1 makes that read fire on crash and cover 2.A fully.

**Consequence for sequencing:** #864 is pulled *into* Pillar 1's critical path (it was previously
"pay-down"). Single-write-path first, then opacity/placement persistence, then fire-restore-on-crash.

---

## 5. Q4 — Can admission control ship independently/early?

**Answer: Yes — already done and proven.** Pillar 3 (commit-aware admission gate) merged in #1853
with zero dependency on the host rework: `sysinfo::available_commit_gb()` + pure
`runner::admit_spawn()` gate in `run_agent`, refusing spawn below
`AGENTMUX_AGENT_COMMIT_RESERVE_GB`. It stops overcommit at the source regardless of whether the
host is disposable yet. Q4 is closed empirically. Remaining Pillar 3 follow-ons (queue-and-drain
instead of hard refuse, per-agent working-set cap, frontend "memory full" badge) are likewise
independent of Pillar 1 and can land anytime.

---

## 6. Resulting Pillar 1 implementation sequence (for the sized spec next)

1. **#864 — collapse layout to one write path** (reducer is sole writer; delete wcore-direct).
   *Hard prerequisite — durability is incoherent with two writers.*
2. **Persist the two host-only topology facts** to srv: per-window opacity, floating-pane
   placement + `last_known_normal_rect`.
3. **Audit window-set completeness** — confirm no host-only window truth beyond what
   `shadow_*` already projects; close gaps.
4. **Fire the cold-start restore path on crash** (mid-session reproject) + the "Restoring session…"
   overlay; ensure in-flight (2.C) work is **re-derived from topology, not resumed**.
5. **E2E test:** "host OOM ⇒ session reprojects" (topology equivalence within budget, overlay
   shown, no orphan tree).
6. **Then the collapses land:** graceful-flush-vs-crash incoherence deleted (one recover path);
   saga layer collapses to an in-memory registry (nothing durable left to compensate).

---

## 7. Risks / honest caveats
- **#864 is now blocking, not optional** — if the layout split-brain proves hairy, Pillar 1 stalls
  behind it. That's the right place for the risk to surface (durability *requires* a single writer).
- **In-flight re-derivation (2.C) is the subtle part** — a reproject that tries to *resume* a
  half-finished create instead of re-deriving from topology reintroduces the orphan class. The
  design rule (derive desired end-state, re-drive idempotently) must be enforced in review.
- **Pools rebuild empty** — a reproject momentarily has cold pools; first new-window/pane after a
  crash is slower until warm. Acceptable (crash-path only), but note it so it isn't "fixed" with
  pool-state serialization (which would re-introduce ephemeral-handle persistence — an anti-goal).
- This remains a **refactor, not a rewrite** — it reuses cold-start restore, the SQLite stores, the
  reducer, and the launcher projection. The work is discipline (one writer, no host-only truth),
  not new machinery.

---

## 8. Sources
- George Candea & Armando Fox, *Crash-Only Software*, HotOS-IX, 2003 —
  https://www.usenix.org/conference/hotos-ix/crash-only-software ·
  https://dslab.epfl.ch/pubs/crashonly.pdf
- George Candea et al., *Microreboot — A Technique for Cheap Recovery*, OSDI 2004 —
  https://www.usenix.org/legacy/event/osdi04/tech/full_papers/candea/candea.pdf ·
  https://en.wikipedia.org/wiki/Microreboot
- *Recovery-Oriented Computing: Building Multitier Dependability* —
  http://www.engr.newpaltz.edu/~bai/EGE534/can04.pdf
- Event-sourcing snapshotting / background write-through latency —
  https://www.kurrent.io/blog/snapshots-in-event-sourcing/ ·
  https://domaincentric.net/blog/event-sourcing-snapshotting
- Internal: `docs/architecture/DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md` §7 (Q1–Q4),
  `docs/specs/SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md` (Pillar 1), `state.rs` inventory,
  issue **#864** (layout single write path).
