# Armory Feature Status — Post-Refactor Audit

> **Archived 2026-07-12.** Superseded — its "gaps still open" finding is stale; PR #2023 (2026-07-08) resolved the remaining catalog gaps it flagged. Consolidated tracking: issue #2024.

**Date:** 2026-07-07
**Baseline:** `agentmux` `origin/main` @ `43f32ceb` (v0.51.0)
**Method:** repo history (`git log --grep`), GitHub issues/PRs/discussions (`gh`), and direct source verification of every claim below — no assumption taken from spec/issue prose alone.
**Verdict: incomplete.** The rename + primitives refactor shipped its foundation cleanly, but a self-declared 7-PR plan stopped at PR 4 of 7, one gap it *did* close is undocumented as closed, and one doc-cleanup item was never done.

---

## 1. What "Armory" is

Armory is the rename of the old **Trust Center** nav tab, expanded to be the catalog surface for three previously-inline primitives — MCP servers, Skills, and Presets (renamed **Bundle**) — pulled out of per-agent inline config into standalone, shareable, globally-bindable records. Two overlapping efforts landed in the same window (2026-07-02 → 07-04):

1. **Trust Center → Armory rename** (UI + docs) — `specs/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md`
2. **Preset → Bundle rename** (App API + UI) — `specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`
3. **MCP/Skills as first-class primitives** — `specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`, executed as a **7-PR plan (A1–A7)** that only ever existed as prose inside PR descriptions, never promoted to a tracked doc until #1960 (see below).

## 2. Commit history — what shipped

| PR | Title | Status | Date |
|---|---|---|---|
| #1910 | docs: Trust Center rename proposal | Merged | 07-02 |
| #1917 | feat(ui): rename Trust Center → Armory | Merged | 07-02 |
| #1913 | docs: Preset → Bundle refactor spec | Merged | 07-02 |
| #1918 | feat(armory): rename Preset → Bundle (App API + UI, phase 2) | Merged | 07-02 |
| #1935 | fix(armory): distinct icon + finish Trust Center comment sweep | Merged | 07-03 |
| #1943 | **A1** — feat(armory): mcp.ts/skill.ts rpc-api bindings | Merged | 07-03 |
| #1944 | A2 (first attempt) — closed, GitHub auto-closed it when #1943's base branch was deleted on merge | Closed (unmerged) | 07-03 |
| #1946 | **A2** — feat(armory): MCP Servers + Skills tabs in the Agent setup modal (re-opened re-file of #1944) | Merged | 07-03 |
| #1948 | **A3** — feat(armory): MCP Servers + Skills catalog tabs (global CRUD) | Merged | 07-03 |
| #1962 | **A4 (unlabeled)** — feat(armory): `bound_to_agent` flag on `mcp.list`/`skill.list` — stateful bind/unbind toggle | Merged | 07-04 |

Nothing touching armory/mcp/skill/bundle has merged since #1962 (2026-07-04). The most recent main commit is #2021 (07-07), three days of unrelated (messaging-bridge) work later.

## 3. Tracking issue: #1960

[**#1960 — "Armory MCP Servers/Skills catalog UI: PRs A4–A7 of the 7-PR plan were never filed"**](https://github.com/agentmuxai/agentmux/issues/1960), opened 2026-07-04 09:02 by agentx, **still OPEN**, zero comments. It documents that A1–A3 shipped, then the author (AgentY-asaf) moved to unrelated Identity/agent-pane work and A4–A7 were never filed. It lists six confirmed gaps.

**Important staleness in the issue itself:** #1960 was filed at 09:02 on 07-04 and lists gap #1 ("bound to me" indicator) as "in progress." PR #1962, which fully fixes gap #1, merged the same day at 09:17 — 15 minutes later. The issue was never updated to check that item off. Anyone reading #1960 today would incorrectly think gap #1 is still open.

I re-verified each of the six gaps directly against current `origin/main` source rather than trusting the issue text:

| # | Gap | Issue says | Actual state on main (verified) |
|---|---|---|---|
| 1 | "Bound to me" indicator on catalog rows / agent modal Bind-Unbind toggle | In progress | **Fixed** by #1962 — `mcp_server_list`/`skill_list` now return `bound_to_agent: bool` (`agentmux-srv/src/backend/storage/mcp_servers.rs`, `skills.rs`); `AgentMcpModal.tsx`/`AgentSkillsModal.tsx` render a real stateful toggle + badge. Issue text is stale. |
| 2 | "Used by N agents" count on Armory catalog rows (required by spec §8) | Not implemented | **Confirmed still missing.** No `used by`/`usedBy`/`agent_count` string or field anywhere in `frontend/app/view/mcp/mcp-manager.tsx`, `mcp-model.ts`, `frontend/app/view/skill/skill-manager.tsx`, or `skill-model.ts`. |
| 3 | No bind-to-agent action from the catalog itself (only from the per-agent modal) | Not implemented | **Confirmed.** `mcp-manager.tsx`/`skill-manager.tsx` are pure CRUD (list/create/edit/delete/toggle-global); no bind affordance. |
| 4 | No deep-linking between agent config view and the catalog | Not implemented | **Confirmed still missing** — required by spec §8 ("deep-linked from an agent's config view... pick existing or create new"), not present in either modal or the catalog view. |
| 5 | Bundle-level grouping (Briefs/Bundle refs) | Unbuilt, intentionally out of scope for now | Confirmed unbuilt; this one is explicitly deferred (spec §9 non-goal for v1), not a regression. |
| 6 | `CLAUDE.md`'s "Not widgets" table never updated for the new MCP/Skills catalog tabs, and still describes the primitive in pre-rename terms | Not fixed | **Confirmed.** `CLAUDE.md:171` still has a row keyed **"Presets"** (not "Bundle" or "Armory catalog") describing "a 'preset'... instructions + context files + MCP servers + skills" with no mention of the now-standalone MCP Servers / Skills catalog tabs that PRs #1943/#1946/#1948 shipped. |

Net: of six tracked gaps, **one is done but unmarked**, **three (catalog used-by-count, catalog-side binding, deep-linking = A4 remainder/A5–A6 territory) are genuinely still open**, **one (Bundle grouping) is correctly deferred**, and **one (docs) is a straightforward leftover cleanup**.

## 4. Other loose ends found while verifying

- **PR #1944** is closed-but-unmerged in the PR list with no comment explaining why unless you open it — it's not a rejected/abandoned attempt, it's a mechanical GitHub artifact (base branch deleted on #1943 merge, so GitHub force-closed it; re-filed cleanly as #1946 with a comment cross-linking). Not a gap, but worth knowing so it isn't mistaken for scrapped work.
- No open PR, branch, or discussion currently continues A5–A7. `git branch -r` / recent branch list shows no `armory`, `mcp`, or `skill`-named in-flight branch beyond the merged ones.
- Related but distinct open issues that touch adjacent surface, not armory itself: **#1624** ("Reconcile per-agent keychain with the live identity-bundle system") and **#678** (Identity System OAuth/vault) — these are Identity-system work, not Armory catalog work; don't conflate them when triaging.

## 5. Recommendation

1. Update #1960 to check off gap #1 (done via #1962) so the tracking issue reflects reality — right now it understates progress.
2. The next real unit of work is gap #2 ("used by N agents"), per #1960's own stated sequencing (items 2–3 both need per-row bind-state data, which #1962 already added the plumbing for on the per-agent side — the catalog-side query is the remaining lift).
3. Gap #6 (CLAUDE.md) is a 10-minute fix and should be picked up opportunistically rather than tracked separately — it's a documentation-accuracy issue, not a feature gap.
4. No urgency on gap #5 (Bundle grouping) — it's correctly scoped out of v1 by the spec itself.
