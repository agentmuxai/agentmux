# Report: "Trust Center" → "Armory" terminology sweep — scan + cleanup scope

**Date:** 2026-07-19
**Author:** Agent2
**Status:** historical — scan complete; scoped cleanup (this report's §3) implemented alongside this report. Broader findings (§4) explicitly out of scope for this pass.
**Trigger:** User: *"There is no trust center anymore, we have an armory pane .. lets first do a scan to clear out the old info."*
**Related:** `docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` (the original rename spec, shipped PR #1917); `docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md` §6 (already found most of this, unaddressed since); `agentmux-docs/specs/AUDIT_DOCS_VS_CODE_2026_07_07.md` P0#1 (same finding, docs-repo side, unaddressed since).

## Summary

The Trust Center → Armory rename (PR #1917, 2026-07-02) covered the user-facing product surface correctly and completely — verified: zero remaining "Trust Center" strings in live frontend/backend code, this repo's root `CLAUDE.md`'s "Not widgets" table already says Armory. What's stale is everywhere *around* the rename: reference docs across three repos that predate it, two internal identifier names the rename spec itself flagged as "optional, low priority" and never got done, and — the most consequential finding — the JEKT security auto-escalation keyword list in **three independent copies across three repos** still keys on `"trust center"` with no `"armory"` counterpart, meaning a message about the Armory's credential surface doesn't reliably auto-escalate to SENSITIVE the way a message about the identically-sensitive old-named surface would have.

This report scopes the fix to exactly the renaming/staleness sweep the user asked for. A much larger, already-documented body of docs work (Bundle semantics rewrite, modal→pane navigation updates, ~15 undocumented shipped features) exists in the two audits cited above — explicitly **not** touched here; see §4.

## 1. What's already correct (verified, no action)

- **Live product code** (`frontend/`, `agentmux-srv/`): zero remaining `"Trust Center"` strings. The one code comment mentioning it (`frontend/app/block/block.tsx:50`) is a *correct* historical note explaining why the `trust → armory` view-key shim exists — appropriately left as-is.
- **This repo's root `CLAUDE.md`**: the "Not widgets" table's Presets/Armory row already says Armory (§H of the rename spec), and — correcting an earlier draft of this report — its own JEKT keyword line never explicitly listed `trust center` in the first place (it trails off with "etc."), so there's no gap to fix in *this specific file*. (There is a separate, non-repo-tracked per-agent `CLAUDE.md` at each agent's local `~/.agentmux/agents/CLAUDE.md` that does list `trust center` explicitly and diverges from this repo's copy — flagged in §2 as a finding, not fixed here, since it isn't version-controlled in any repo this pass touches.)
- **`agentmux-docs`**: the actual page rename already shipped (`trust-center.md` deleted, `armory.md` created, `astro.config.mjs` has the `/trust-center → /armory` redirect). Five pages (`glossary.md`, `identity.md`, `main-menu.md`, `pane-types.md`, `armory.md` itself) correctly say **"Armory (formerly Trust Center)"** — a deliberate, well-written callout for readers searching the old name, not staleness. No action needed on any of these six.
- **Historical/changelog docs** (`VERSION_HISTORY.md`, and — see §3.3 — two docs archived by this pass): "Trust Center" mentions here are accurate records of what the product was called *at the time the entry was written*. Rewriting them to say "Armory" would misrepresent history. Correctly left untouched.

## 2. The real gap: JEKT sensitive-keyword lists (stale in three places, plus one non-repo file)

JEKT security rules auto-escalate a message to SENSITIVE when it contains certain keywords — `account.key.verify`, `keychain`, `trust center`, etc. — on the theory that content about credential-management surfaces warrants a human pause before an agent acts on it. Several independent implementations/documentations of this same keyword list exist. Checked each directly; three are real, repo-tracked gaps:

| File | Repo | List name | Status |
|---|---|---|---|
| `agentmux-srv/src/backend/reactive/sanitize.rs:157` (`SENSITIVE_SUBSTRING_KEYWORDS`) | agentmux | code | Has `"trust center"`, missing `"armory"` |
| `docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md:144` (the canonical spec `REPORT_REPO_HEALTH_AUDIT` names as source-of-truth) | agentmux | doc | Has `trust center`, missing `armory` |
| `muxbus/server/src/index.ts:254` (`SENSITIVE_KEYWORDS`) | agentmux-cloud | code | Has `'trust center'`, missing `'armory'` |
| `src/content/docs/internals/interagent-comms.md:141` | agentmux-docs | doc | Has `trust center`, missing `armory` |

Practical effect: a jekt message discussing the Armory's credential UI (e.g. "go add a PAT in the Armory") gets substring-matched against `pat`/`keychain`/etc. and likely still escalates via *those* keywords today — this isn't a total blind spot — but the specific, deliberate "this surface is sensitive" signal the original authors encoded no longer fires for its current name. `docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md` §6.2 found this exact drift on 2026-07-05 (also noting the `agentmux-cloud` matcher is naive-substring rather than whole-word, a separate, pre-existing bug — see §4). Unaddressed since.

**One further, out-of-repo finding, not fixed here:** each agent's local, non-git-tracked `~/.agentmux/agents/CLAUDE.md` (distinct from this repo's own root `CLAUDE.md` — the two have diverged) explicitly lists `trust center` as a keyword too. Since it isn't checked into any of the three repos this pass touches, there's no PR to open for it — flagging it here in case whoever maintains that file's source/deploy process wants to sync it.

**Fixed in this pass** (§3.4): added `armory` alongside `trust center` in the three repo-tracked locations (additive — strictly increases escalation coverage, changes no existing behavior).

## 3. Cleanup implemented in this pass

### 3.1 Terminology fix — live/Draft specs (agentmux repo)

12 non-archived specs, all `Status: Draft/Planned/In progress/Proposal` (i.e. still-current reference docs, not historical narrative), had incidental stale `"Trust Center"` mentions corrected to `"Armory"`:

`docs/specs/REPORT_AUTH_ARCHITECTURE_2026_06_25.md`, `SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md`, `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md`, `SPEC_AGENT_PICKER_TILE_GRID_2026_06_17.md`, `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md`, `SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md`, `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`, `SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20.md`; `docs/specs/PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md`, `docs/specs/SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md`, `docs/specs/SPEC_SETTINGS_PANE_2026_06_25.md`, `docs/specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`.

`PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md` needed the most judgment: its title and a whole section header ("Trust Center information architecture") are structurally built around the old name, updated to Armory — **except** a direct driver quote at line 6 (*"I'd like a cleaner model… break out skills and MCP into the Trust Center…"*), left verbatim since it's a historical quote, not descriptive prose.

`docs/specs/SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md` had zero matches for the literal phrase `"Trust Center"` — correction, post-review: it did have a stale identifier, `trustCenter: opts.onTrustCenter`, in an embedded code sample. This was missed because the scan for §3.5's code-identifier cleanup only checked actual `.ts`/`.tsx` source, not identifier references inside doc code samples. Fixed alongside this report's own correction; see §6 for the review-driven fixes this file's first draft required.

`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` gets different treatment for its two kinds of mention: prose describing the UI surface ("register a GitHub PAT in the trust center keychain") → "Armory", but its keyword-list line (§2 above) gets `armory` *added alongside* `trust center`, not replaced — consistent with the additive, no-regressions approach used for the other three keyword lists.

### 3.2 Terminology fix — agentmux-docs

Only one genuine gap after the above verification: `internals/interagent-comms.md:141`'s user-facing keyword-list documentation (see §2/§3.4).

### 3.3 Archived two fully-historical docs

Two docs are session narratives / "already fixed" reports describing a point in time, not live reference material — moved to this repo's existing `archive/` convention with an archived-header note, **text left unchanged** (accurate as history, per the same principle as VERSION_HISTORY.md):

- `docs/analysis/ANALYSIS_ACCOUNTS_UI_GAPS_2026-06-18.md` (Status: "Bugs fixed in this session") → `docs/analysis/archive/`
- `docs/handoff/HANDOFF_MEMORY_IDENTITY_MODALS_2026_06_19.md` (session handoff, PRs already merged) → `docs/archive/` (matching where prior `HANDOFF-*.md` files already live)

### 3.4 JEKT keyword lists — added `armory`

Additive fix (old keyword kept) in the three repo-tracked locations from §2: `sanitize.rs`, `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` (folded into the §3.1 pass), `agentmux-docs/internals/interagent-comms.md`, and `agentmux-cloud/muxbus/server/src/index.ts`.

### 3.5 Code identifier cleanup (agentmux repo) — the rename spec's own deferred items

`docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` §E/§G explicitly flagged these as "optional... in the same PR" and they were never done:

- `frontend/app/view/accounts/accounts-manager.tsx:39` — synthetic ViewModel scope id `"trust-center:accounts"` → `"armory:accounts"`.
- `onTrustCenter`/`trustCenter` prop and variable names in `frontend/app/view/agent/agent-view.tsx`, `frontend/app/view/agent/failure/failure-accessory.ts` (+ its test), `frontend/app/view/agent/hooks/useAgentFailure.ts` (+ its test) — renamed to `onOpenArmory`/`openArmory`. The user-facing *label* strings here (`"Armory → Accounts"`, `"Armory (switch / upgrade)"`) were already correct; only the internal plumbing names were stale.

## 4. Found but explicitly out of scope for this pass

Two existing, already-written audits document substantially more docs staleness than the Trust Center naming issue alone. Not touched here — flagged for a separate, deliberate pass:

- **`docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md`** — a 23-item, 5-tier action plan spanning dead-code deletion, BUILD.md rewrites, `agentmux-cloud` README staleness ("Status: exploratory. No code yet" while a deployed Fastify server/CDK stack exists), and the naive-substring vs. whole-word JEKT keyword matcher inconsistency between repos (§6.2, distinct from the missing-`armory` gap this report fixes).
- **`agentmux-docs/specs/AUDIT_DOCS_VS_CODE_2026_07_07.md`** — P0 items 2-5 (Bundle semantics need a structural rewrite, not find-replace; Settings/Toolchain/Armory modal→pane navigation instructions; per-agent pane-header icon consolidation; `identity.*`/`preset.*` App API namespaces documented as "planned" when they've shipped under different names) and P1's ~13 shipped-but-undocumented features (Cron/Loop MCP tools, ghost-text suggestions, MuxBus Cloud sign-in chip, etc.).

Both remain accurate scoping documents for whoever picks up that broader work next.

## 5. Files changed in this pass

| Repo | File | Change |
|---|---|---|
| agentmux | `agentmux-srv/src/backend/reactive/sanitize.rs` | Add `"armory"` to `SENSITIVE_SUBSTRING_KEYWORDS` |
| agentmux | `frontend/app/view/accounts/accounts-manager.tsx` | `"trust-center:accounts"` → `"armory:accounts"` |
| agentmux | `frontend/app/view/agent/agent-view.tsx`, `failure/failure-accessory.ts(+.test.ts)`, `hooks/useAgentFailure.ts(+.test.ts)` | `onTrustCenter`/`trustCenter` → `onOpenArmory`/`openArmory` |
| agentmux | 12 specs under `docs/specs/` and `specs/` (§3.1) | `"Trust Center"` → `"Armory"` (contextual, quote-preserving) |
| agentmux | `docs/analysis/ANALYSIS_ACCOUNTS_UI_GAPS_2026-06-18.md` | Archived → `docs/analysis/archive/` |
| agentmux | `docs/handoff/HANDOFF_MEMORY_IDENTITY_MODALS_2026_06_19.md` | Archived → `docs/archive/` |
| agentmux-docs | `src/content/docs/internals/interagent-comms.md` | Add `armory` to documented keyword list |
| agentmux-cloud | `muxbus/server/src/index.ts` | Add `'armory'` to `SENSITIVE_KEYWORDS` |

## 6. Corrections made during review

The original scan methodology (single-line `grep -i "trust center"`) had two blind spots, both caught by ReAgent's PR review rather than a second self-check:

1. **Word-wrapped prose.** `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md`'s Phase 5 section had "...identity from the trust\ncenter." — the phrase split across a markdown line wrap, invisible to a single-line grep. A follow-up multiline-aware search (`trust\s*\n\s*center`) across the whole repo found exactly one more instance of the same class, in `PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md` ("...home in the Trust\nCenter."). Both fixed; the multiline search now returns zero hits repo-wide.
2. **Identifiers inside doc code samples.** §3.5's code-identifier cleanup (`trustCenter`/`onTrustCenter` → `openArmory`/`onOpenArmory`) only searched actual `.ts`/`.tsx` source files. `SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md` has an embedded code sample using the same old identifier (`trustCenter: opts.onTrustCenter`) that the source-only search never saw — this is also what produced §3.1's now-corrected, originally-mistaken claim that this file had "zero actual matches." Fixed; a follow-up repo-wide grep for the bare identifiers confirms the only remaining matches are this report's own narrative and the correctly-untouched archived rename spec.
