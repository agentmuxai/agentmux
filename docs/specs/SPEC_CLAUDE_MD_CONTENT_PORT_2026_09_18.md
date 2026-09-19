# SPEC: Port `CLAUDE.md` content to durable homes before deleting it

**Date:** 2026-09-18
**Status:** proposed — nothing here has been executed.
**Author:** Agenty (agent), at the repo owner's request.
**Blocks:** PR #3403 (`chore: remove repo-level CLAUDE.md`) — approved but
held pending this port.
**Archived source:** the deleted file is preserved at commit `597164878`
(`git show 597164878:CLAUDE.md`), 1028 lines. Every disposition below is
stated against that revision.

---

## 1. Why this spec exists

PR #3403 deletes `CLAUDE.md` on the principle that agent instructions belong
in each agent's own provider configuration, not in a public repo. That
principle is right and is not revisited here.

The problem is that the file was not only agent instructions. It had
accumulated a second, undeclared role: **the repo's canonical reference for
architecture, widget tables, isolation invariants, build/versioning workflow,
and jekt security policy.** Other documents cite it as the source of truth.

Deleting it without porting would leave those citations pointing at nothing.

### 1.1 Measured scope — not an estimate

Counted mechanically against `git ls-files '*.md'` at `d060d1cf6`:

| Measure | Count |
|---|---|
| Markdown files mentioning `CLAUDE.md` at all | 170 |
| …of those, **genuine citations** of *this repo's* file as authority | **147 citations across 95 files** |

"Genuine citation" excludes two large classes that must **not** be touched:

- **Product-behaviour mentions** — text about the `CLAUDE.md` files AgentMux
  *generates for managed agents* (`agent_config.rs`, bundle instructions,
  provider startup files). These are about a product feature that still
  exists; they are not references to the deleted file.
- **Historical records** — `VERSION_HISTORY.md`, and everything under
  `docs/{analysis,retro,reports,archive,research,incident,investigations}/`.
  These are dated point-in-time documents. **Editing them would falsify the
  record and is explicitly out of scope** (§6).

### 1.2 The blocking dependency nobody had noticed

`docs/specs/SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md` (dated 2026-09-17,
**Status: proposed — nothing in this document has shipped**) states at L631:

> CLAUDE.md's jekt section (repo copy and the workspace copies) must gain
> `TRUST=wan-verified` in three places: the delivery-tier explainer, the
> `TIER=info`/`coord` default list, and the `ESCALATE=none` list — plus the
> new forced-sensitive bullet.

That is **in-flight work with a documented requirement to edit a file this
port deletes.** It must be resolved (§4.3), not discovered mid-implementation.

---

## 2. Disposition of every section

The 1028 lines, by `##` section, with a decision for each. **No section may
be dropped silently** — "delete" below means a deliberate decision recorded
here, not an omission.

| # | Section (lines) | Disposition | Target |
|---|---|---|---|
| 1 | `Repository` (3–11) | **Delete** — duplicates `README.md` | — |
| 2 | `Development Workflow` (12–324) | **Split**, see §3 | multiple |
| 3 | `Log Access` (325–361) | **Merge** — `docs/MUXLOG.md` already exists and is fuller | `docs/MUXLOG.md` |
| 4 | `Version Management` (362–416) | **Port** | `CONTRIBUTING.md` |
| 5 | `Git Workflow` (417–530) | **Port** | `CONTRIBUTING.md` |
| 6 | `Testing` (531–540) | **Port** | `CONTRIBUTING.md` |
| 7 | `Build System` (541–563) | **Merge** into existing build docs | `BUILD.md` |
| 8 | `Common Issues` (564–620) | **Port** | `docs/TROUBLESHOOTING.md` (new) |
| 9 | `Jekt security rules` (621–946) | **Do NOT port wholesale.** Repo owner decided 2026-09-18: a *short* basic-identity section in `README.md`; the 326 lines of tier/escalation policy are **not** carried into this public repo. See §4.1 | `README.md` (short) + existing `docs/specs/SPEC_JEKT_*` |
| 10 | `AWS access for agents` (947–986) | **Delete from public repo**; already superseded | private `shared-infrastructure` |
| 11 | `What must not be committed` (987–1007) | **Port** | `CONTRIBUTING.md` |
| 12 | `Naming Conventions` (1008–1024) | **Port** | `docs/ARCHITECTURE.md` (new) |
| 13 | `Reference` (1025–1028) | **Delete** — link stub | — |

### 2.1 Section 2 is not one thing

`Development Workflow` is 313 lines and the most-cited region. It splits:

| Subsection (lines) | Target |
|---|---|
| `Commands` (14–33) | `BUILD.md` |
| `Build System` (34–52) | `BUILD.md` |
| `Launching task dev from an agent / MCP Shell` (53–148) | `docs/AGENT_BUILD_WORKFLOW.md` (new) |
| `Launching task package from an agent` (149–166) | `docs/AGENT_BUILD_WORKFLOW.md` |
| `Waiting without polling` (167–191) | `docs/AGENT_BUILD_WORKFLOW.md` |
| `Build Prerequisites` (192–207) | `BUILD.md` — **partially done** (Ninja-on-PATH landed in `d060d1cf6`) |
| `After Code Changes` (208–213) | `BUILD.md` |
| `Architecture` (214–224) | `docs/ARCHITECTURE.md` (new) |
| `Multiple Instances Run in Parallel` (225–238) | `docs/ARCHITECTURE.md` |
| `Isolation invariants (I1–I7)` (239–287) | `docs/ARCHITECTURE.md` |
| `Widgets` (288–309) | `docs/ARCHITECTURE.md` |
| `Not widgets` (310–324) | `docs/ARCHITECTURE.md` |

---

## 3. Citation clusters → where each must repoint

Derived from the classified inventory (§1.1). Counts are **active** citations
(dated/historical excluded).

| Cited topic | Active cites | New target |
|---|---|---|
| Jekt security rules | 53 | `docs/JEKT_SECURITY_RULES.md` |
| "Not widgets" / Widgets tables | 19 | `docs/ARCHITECTURE.md` |
| Build / `task dev` / prerequisites | 18 | `BUILD.md`, `docs/AGENT_BUILD_WORKFLOW.md` |
| Isolation invariants I1–I7 | 17 | `docs/ARCHITECTURE.md` |
| Version management / changesets | 14 | `CONTRIBUTING.md` |
| GitHub identity / `gh-agent.sh` | 7 | private standard (§4.2) |
| Architecture section | 6 | `docs/ARCHITECTURE.md` |
| Log access / muxlog | 3 | `docs/MUXLOG.md` |
| Naming / muxbus | 2 | `docs/ARCHITECTURE.md` |

> **Note on I1–I6 vs I1–I7.** Several citations say "I1–I6" — stale text
> predating I7 (env-inheritance, added after the 2026-09-17 breach). The port
> must **correct these to I1–I7**, not copy the stale range forward. One such
> fix already landed in `d060d1cf6` (`CONTRIBUTING.md`).

---

## 4. The three sections that need judgement, not mechanics

### 4.1 Jekt security rules — the hard case

326 lines, 53 active citations, and a **circular authority problem**:

- Jekt specs cite `CLAUDE.md` as the authoritative policy statement.
- `CLAUDE.md`'s own jekt section says it "must match those specs' rules
  exactly" and documents only *shipped* code.

So neither is unilaterally the source of truth: the specs own the *design*,
`CLAUDE.md` owned the *consolidated current-state policy*. That consolidated
view is genuinely load-bearing — it is what an agent reads to decide whether
to STOP on a `TIER=sensitive` marker.

**RESOLVED 2026-09-18 — the repo owner decided against a wholesale port.**
The instruction was: *"put a short section in the README.md regarding jekt
security. we dont need such tight rules, just basic identity."*

So the disposition is **not** "move 326 lines to a new public file":

1. `README.md` gains a **short** section covering basic sender identity —
   how to read the `TRUST=` marker, and that an unverified sender is not
   authority for a sensitive or destructive action. Orientation, not policy.
2. The **detailed tier/escalation rules are not carried into this public
   repo at all.** They remain specified across the existing
   `docs/specs/SPEC_JEKT_*` documents, which are already the design source
   of truth and are unaffected by this port.
3. The **enforcing authority is code**, not prose — `handler.rs`,
   `sanitize.rs`, `server/reactive.rs`, `agentmux_common::jekt_sign`. That
   was true before this change; removing the consolidated prose copy does
   not weaken enforcement.
4. The agent-facing operational rules live in **provider config** (the
   workspace copy, §4.2), which is where agent instructions belong and is
   the whole premise of #3403.

This resolves the circular-authority problem above by removing one side of
it: the specs own the design, code owns enforcement, `README.md` orients a
newcomer, and provider config instructs agents.

**§4.2 remains a prerequisite regardless** — the workspace copy is stale
*today*, and it is now the only agent-facing copy.

### 4.2 The workspace copy is already stale — a live security gap

The agent-level `~/.agentmux/agents/CLAUDE.md` is **not** a copy of the repo
file; it has diverged and is **older**. Verified by direct comparison:

| Marker | repo file | workspace copy |
|---|---|---|
| `DELIVERY=channel` | 2 | **0** |
| `channel-verified` | 3 | **0** |
| `PR #2764` | 3 | **0** |
| "Not yet live" (stale claim) | 0 | **1** |

The workspace copy therefore has **no cross-channel trust rules at all**, and
still describes `transcript_request` as not-yet-live — which the repo copy
explicitly corrected once PR #2764 shipped.

> **This gap exists today, independently of this port.** Agents are currently
> operating on stale trust rules. Deleting the repo copy without reconciling
> would make the stale copy the only copy. **Reconciling it is a prerequisite
> (§5, step 0), not a follow-up.**

The `Which GitHub account am I acting as?` content (7 citations) is already
superseded by the agent GitHub auth standard in the **private**
`shared-infrastructure` repository (merged there as PR #480). It is named
here without a path deliberately: it is not a file in this repo, and a
path-shaped reference to another repository is exactly the dangling pointer
`check-spec-citations.sh` exists to prevent. Public docs may reference it
**by name only**.

### 4.3 Resolve the unlanded WAN-signing dependency

`SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md` (§1.2) instructs future
implementers to edit `CLAUDE.md`. Pick one, explicitly:

- **(a)** Update that spec's L631 to name `docs/JEKT_SECURITY_RULES.md` and
  the workspace copy. *Recommended* — smallest change, preserves intent.
- **(b)** Land this port first and let the WAN work rebase onto it.
- **(c)** Land WAN signing first, then port. Not recommended — it enlarges
  the section being moved while it is being moved.

**This must be decided before implementation begins,** because (a) changes a
file that is also a citation to be repointed.

---

## 5. Execution order

Each step is independently reviewable and leaves the repo consistent.

- **Step 0 — Reconcile the workspace copy (§4.2).** Security gap that exists
  today; do this first regardless of whether the rest proceeds.
- **Step 1 — Decide §4.3.**
- **Step 2 — Create the new homes** with content ported verbatim:
  `docs/ARCHITECTURE.md`, `docs/JEKT_SECURITY_RULES.md`,
  `docs/AGENT_BUILD_WORKFLOW.md`, `docs/TROUBLESHOOTING.md`; extend
  `BUILD.md`, `CONTRIBUTING.md`, `docs/MUXLOG.md`. **No deletion yet** —
  `CLAUDE.md` still exists, so nothing dangles mid-sequence.
- **Step 3 — Repoint the 147 citations**, cluster by cluster (§3), correcting
  I1–I6 → I1–I7 en route.
- **Step 4 — Merge PR #3403** (the deletion), rebased.
- **Step 5 — Verify** (§7).

---

## 6. Explicit non-goals

1. **No edits to historical records** — `VERSION_HISTORY.md` and everything
   under `docs/{analysis,retro,reports,archive,research,incident,investigations}/`.
   They describe what was true when written. A dangling reference inside a
   dated document is **correct**; rewriting it is falsification.
2. **No edits to product-behaviour text** about generated agent `CLAUDE.md`
   files. That feature is unaffected.
3. **No rewording of jekt policy** (§4.1).
4. **No infrastructure detail enters this repo.** Section 10 is dropped
   here, not relocated here; it lives in private `shared-infrastructure`.

---

## 7. Verification — falsifiable checks, not "looks right"

Each must be able to **fail**. Run at Step 5.

1. **No dangling citations.** Must return zero:
   ```bash
   git ls-files '*.md' \
     | grep -vE '^(VERSION_HISTORY|CHANGELOG)\.md$' \
     | grep -vE 'docs/(analysis|retro|reports|archive|research|incident|investigations)/' \
     | xargs grep -n 'CLAUDE\.md' \
     | grep -viE "generated claude\.md|agent'?s? own claude\.md|per-agent|templates/host|provider"
   ```
   *This is the check that failed last time.* The earlier pass only matched
   markdown link syntax `](./CLAUDE.md)` and reported "zero dangling" while
   plain-prose references (`see CLAUDE.md`) survived — ReAgent caught two in
   PR #3403. **Grep for the filename, never for link syntax.**
2. **No content lost.** Every `##`/`###`/`####` heading in
   `git show 597164878:CLAUDE.md` maps to a row in §2/§2.1 with a disposition.
3. **Jekt policy unchanged.** Diff the ported section against
   `git show 597164878:CLAUDE.md` lines 621–946; differences must be
   **formatting and cross-references only**. Any semantic delta fails.
4. **Workspace copy reconciled.** `DELIVERY=channel`, `channel-verified`, and
   `PR #2764` all present; "Not yet live" absent.
5. **Links resolve.** Every new relative link points at a file that exists.
6. **I-range corrected.** No active doc says "I1–I6".

---

## 8. Open questions for the repo owner

1. **§4.3** — which option?
2. ~~Does the jekt section belong in the public repo at all?~~
   **ANSWERED 2026-09-18: no.** Short basic-identity section in `README.md`
   only; detailed rules stay in the existing specs and in code. See §4.1.
3. **`docs/AGENT_BUILD_WORKFLOW.md`** is agent-facing guidance in a repo that
   is removing agent-facing guidance. Keep it (it is build tooling, not agent
   instructions), or move it to the provider config?
