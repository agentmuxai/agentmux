# Spec: Swarm, short colour-coded tags for each agent's model and seat

**Status:** implemented: PR A (the seat colours, the tag component and its colour tests; #7956) and PR B (the Swarm row reads the model facts #48222 P1 added to the runtime row, so the model tag shows). Other instances' agents get a model tag when #48222 P2 returns their facts, with no frontend change beyond reading the same fields. Approved with D1 to D5 as recommended, 2026-11-20. Task #48219 (spec), build #48220. Author: Agent1@narko; build: Tansy@narko.
Depends for data on #48221 (MuxAPI agent live facts, `SPEC_MUXAPI_AGENT_LIVE_FACTS_2026_11_20.md`, build #48222 P1).

## 1. Requirement

In the Swarm, every agent row says at a glance which model the agent is running now and what seat it holds, each as a short tag whose colour tells the family or role. A person scanning the list can pick out "the Opus agents" or "the leaders" by colour, and can still read the word when they cannot tell the colours apart. The model is what the agent is actually running (AgentMux's own runtime state), never what its definition says.

## 2. What exists today (read from the code, 2026-11-20)

| Piece | Where | Notes |
|---|---|---|
| Seat badge | `swarm-seats.tsx` `SwarmSeatPill`, a neutral `Pill` with text from `seatBadgeText` ("Leader · product") and the tooltip from `seatTooltipLine`; also on other instances' rows (`remoteSeatInfo`) | Sits in the header row right after the name, inside `.swarm-seat-slot` (shown whole or cut with an ellipsis down to 4ch, never as a stub), and drops at a pane width of 240 px (`swarm-view.scss`, container `swarm-tree`). All grey today. |
| Same seat words elsewhere | My Agents tile (`MyAgentsRow.tsx`), Atlas, the command palette entries | All go through `seat-format.ts`, so they read the same everywhere. |
| Header row | `swarm-agent-card.tsx` `AgentCardHead`, 36 px high, one line | Select box, chevron, provider icon, name, seat badge, "No pane", unseen dot, unread chip, context size, last active, notes, status chip, Supervisor chip. A step ladder by pane width drops parts in turn: age and Supervisor word at 360, context size and Supervisor mode at 320, name floor at 280, seat badge at 240, clip at 200. |
| Line under the row | `.swarm-activity-summary`, padding-left 30 px, wraps | Never empty. |
| Model | **Not in the row's data.** `AgentRuntime` (`agentruntime.list`, the Swarm's source of stopped and parked agents) has no model field. The per-pane `agentruntime` event carries `effective_model`, but only for control-protocol panes and only on this host. | #48221 adds `model_label`, `model_source` (`effective`, `observed`, `requested`, `unknown`) and `pending_switch` to the `AgentRuntime` row (its section 5.3) and says the Swarm tag reads them. |
| Tag primitive | `element/ui/Tag.tsx` (`tag.scss`, `tag-tokens.ts`, contrast measured per theme in `tag-contrast.test.ts`) | Tones are the five semantic ones (neutral, ok, warn, error, accent). The wash is the colour at 14% over the surface, the outline an `-edge` token held at 3:1, the text the main text colour at 4.5:1 on the wash. A tag carries its meaning in words, never in colour alone. |
| Precedent for a themed set of named colours | Kanban stage colours: `--kanban-stage-*` tokens aliased onto existing theme tokens, overridden per theme only where two meet, measured by `kanban-stage-colours.test.ts` (every pair at least 9 apart in OKLab x100, at least 3:1 against the surface) | The same method fits here. |

## 3. Where

### 3.1 Recommendation

- **Seat tag: stays where it is** (header, after the name, same slot and same drop at 240 px). It changes only in colour (and in being a `Tag`, section 6). No layout change, so none of the row's width ladder moves.
- **Model tag: a new line under the header row**, between the header and the summary line, left-aligned with the name (same 30 px indent as the summary), one tag high, not wrapping. It exists only for an agent whose model is known. It is not in the header, because at the usual 300 px pane the header is already full by design (name floor, seat badge, status chip, Supervisor chip) and a 70 px tag would either push the seat badge out or go itself. The tags line is always visible at every width down to 120 px, where it is cut with the summary line.
- Cost: an agent with a known model gains one line (about 20 px at 100% chrome). A list of 75 agents is about 1,500 px longer. See decision D1 for the alternatives and the toggle.

### 3.2 Other surfaces (proposed, not widened without a yes)

| Surface | Proposal |
|---|---|
| Agent pane header | **No.** It already shows the provider mark and the Runtime menu names the model (as requested, with the process drift check). A second, coloured model mark would compete with them. |
| Agents conversations / `Agents list` / `Agents status` | **No tags** (they are text for agents). They get the model as text from #48221 (`model_label` in the row, a column in `Agents status`). |
| My Agents tile, Atlas, command palette | **Seat colour: yes for My Agents and Atlas** (they already show a seat badge through `seat-format.ts`, so a shared `SeatTag` makes the colour the same everywhere, which that module exists to guarantee). The palette entry is plain text and stays plain. **Model tag: no** (a stopped definition's model is a request, not a running fact). Decision D4. |
| Stopped / "No pane" rows in the Swarm | Seat tag as for any row. Model tag shows for a running no-pane agent (it has runtime facts), not for a stopped one (nothing is running; section 5 says what stays). |

## 4. The model tag

### 4.1 Text

The text is the short display label srv gives (`model_label`, for example "Sonnet 5.5", "Opus 5.5", "Fable 5.2", "Haiku 5.5", "GPT 6", "Gemini 3 Pro"). The frontend does not parse versions out of ids when srv gives a label; it only shortens:

- strips a leading "Claude ", trims, and cuts at 16 characters with an ellipsis (the full label is in the tooltip);
- when only an id is known (a peer on an older build, `model_label` missing), a small fallback turns `claude-sonnet-5-5` into "Sonnet 5.5": drop the `claude-` prefix and a trailing date (`-20250929`) or `-latest`, capitalise the first word, join digit groups with a dot. An id the fallback does not recognise shows as written, cut at 16. A test table pins the cases (Claude ids, `gpt-6-astra`, `gemini-3-pro`, a route id like `openrouter/vendor/model`, a local `llama3.1:8b`).
- no effort, route or account in the text. They are in the tooltip.

### 4.2 Family and colour

One colour per family, one family per Claude tier, because the question people ask ("is anyone not on Sonnet?") is per tier and a lighter or darker step of one hue is too close to read across a list. This differs from the brief's "hue per family with a step for tier"; D2 records it.

| Family (matched on the label or id, case-insensitive) | Token | First alias in the default theme (the build tunes it against the test, section 6.3) |
|---|---|---|
| Fable (`fable`) | `--model-fable` | `--term-bright-magenta` |
| Opus (`opus`) | `--model-opus` | `--conn-icon-color-6` (orange) |
| Sonnet (`sonnet`) | `--model-sonnet` | `--term-blue` |
| Haiku (`haiku`) | `--model-haiku` | `--term-cyan` |
| OpenAI (`gpt`, `codex`, `o3`/`o4` style ids, provider `codex`) | `--model-gpt` | `--term-green` |
| Gemini (`gemini`, provider `gemini`) | `--model-gemini` | `--conn-icon-color-2` (violet) |
| Anything else: a route model, a local model, an unrecognised Claude | `--model-other` | `--secondary-text-color` (neutral) |

"Unknown" has no tag (4.3). The tag is a `Tag` with a small left glyph that differs by family group so colour is never the only cue: a spark for Claude tiers, a hexagon for OpenAI, a diamond for Gemini, a plain dot for other. The text is the primary cue, the glyph the second, the colour the third.

### 4.3 States (from #48221's `model_source` and `pending_switch`)

| Data | Shows | Tooltip |
|---|---|---|
| `effective` | The tag in its colour, solid outline | "Sonnet 5.5, running now (confirmed by its CLI). Effort high." |
| `observed` (the last call's model; harnesses that cannot answer `get_settings`) | Same as effective | "Sonnet 5.5, from its last call." |
| `requested` | **Muted**: dashed outline, no wash, secondary text colour, the family glyph still shown | "Sonnet 5.5 was asked for; not confirmed yet." |
| `pending_switch` (a switch just asked for, waiting for the next turn boundary) | Muted as `requested`, with an arrow glyph before the label | "Switching to Opus 5.5 at its next turn. Now: Sonnet 5.5." |
| `route` (an agent on a model route: its gateway's model; #48222 P1) | Same as `effective` | "GPT 6, on its model route." |
| `unknown` with no label, or the agent has no runtime | **No tag, no placeholder, no line.** Nothing is guessed (matches #48221 section 7) | none |
| `unknown` with a label (#48222 sends the label only when it has one) | **Muted**, as `requested` | "... was asked for; not confirmed yet." |
| Differs from the definition (`differs_from_definition`) | Normal tag, no extra mark in the row | A second line: "Its definition says Haiku 5.5." (decision D3 if you want a marker in the row) |
| Stopped agent | None. The remembered request is a definition fact, not a running one | none |

The tag follows `agentruntime:changed:<agent id>`, which #48221 fires when the model, effort, turn state or account change. There is no timer of its own.

### 4.4 Where the facts come from

The Swarm reads the model from the runtime row (`model_id`, `model_label`, `model_source`, `effort`, `pending_switch`, `definition_model_label`, #48222 P1), through `readModelFacts`, the one place it does. The per-pane `agentruntime` event's effective model is not read as a second source of the same fact (#48221's principle is one builder). A row with neither a label nor an id draws no tag.

### 4.5 Other instances' agents

An agent on a LAN peer gets a model tag when #48221's P2 (`agents.status` over the LAN) returns its facts. Until then, and for a peer on an older build or a relay-only agent, it has no tag, like any unknown. Its seat tag works today (`remoteSeatInfo`).

## 5. The seat tag

- Text: unchanged, `seatBadgeText` ("Leader", "Specialist · pane", "Platform tester"). Role plus the qualifier, as the badge shows now, cut by the slot rules already in `swarm-seats.scss` (ellipsis, 4ch floor, gone below 240 px). Nothing about the text changes.
- One colour per role:

| Role | Token | First alias |
|---|---|---|
| Leader | `--seat-leader` | `--accent-color` |
| Super agent | `--seat-super` | `--attention-color` |
| Specialist | `--seat-specialist` | `--term-cyan` |
| Platform tester | `--seat-platform-tester` | `--term-yellow` |
| Analyst | `--seat-analyst` | `--info-color` |
| Worker | `--seat-worker` | `--term-blue` |

- No seat: **no tag** (as today). A muted "no seat" tag on every unseated row would be noise; the "No seat" group heading already names them when the list is grouped by lead. An unknown role (a newer peer) shows as written in the neutral colour.
- The seat tag stays a pill (full radius). The model tag is a rounded rectangle (`--radius-sm`) with its glyph, so a Leader tag and an Opus tag in similar colours still differ by shape and position, not only by hue.

## 6. How it is built (for #48220)

### 6.1 Components and tokens

- `Tag` gains one prop, `hue`: a name from a new `TAG_HUES` table in `tag-tokens.ts` (the 6 seat and 7 model names). It sets `--tag-tone: var(--<name>)` and `--tag-edge: var(--<name>-edge)`; the fill and text rules are the tone ones, unchanged. A tag has a `tone` or a `hue`, never both. The tone set and every other call site are untouched.
- `theme.scss` defines the 13 tokens as aliases (the table above) and an `-edge` token for each (equal to the colour, like `--accent-edge`; a theme overrides only the edge where the colour is under 3:1). Themes override an alias only where the test below says two meet.
- `seat-format.ts` gets `seatHue(role)`; a new `model-tag.ts` has `modelTag({ label, id, provider, source, pendingTo })` returning `{ text, family, hue, muted, glyph, tooltip } | null`. `SwarmSeatPill` becomes a `Tag` with the role's hue; `SwarmModelTag` is new, used in `AgentCardHead`'s new tags line and nowhere else in the first PR.
- The Swarm reads the model through the same `sharedAgentRuntime()` row it already reads for stopped agents, keyed by agent id, so no new subscription.

### 6.2 Narrow widths and zoom

- The tags line is one nowrap flex line. The model tag shrinks by cutting its text with an ellipsis down to its glyph and 4ch, and is gone below 120 px (the same width where the summary line becomes one clipped line), whole or not at all, like the seat badge's slot.
- Sizes are in the units the seat slot uses (`var(--text-xs)`, `1lh`, `var(--space-2)`, 1 px borders), so the tags scale with chrome zoom and nothing is a fixed pixel width. The tags line's own padding uses the summary line's, so the indent follows the name at every zoom.
- The 36 px header height and its ladder are not changed.

### 6.3 Colour and accessibility

- A test, modelled on `kanban-stage-colours.test.ts`, measures in **every theme**: the 7 model colours pairwise at least 9 apart in OKLab x100, the 6 seat colours the same, each colour at least 3:1 against `--main-bg-color` **and** against `--highlight-bg-color` (the selected row's background), and each tag's text at least 4.5:1 on its wash (by extending `tag-contrast.test.ts` to the hue table). The two sets are not required to be apart from each other: they differ by shape, glyph and position. A theme where two meet overrides an alias in its own file.
- Known tight case: the high-contrast theme already has two Kanban stage colours too close (task #48034). The test will show whether the same happens here; the fix goes in that theme's file, with the alias names unchanged.
- Palette (12 hues plus Black and White): these are pane identity colours (the card's border and selection ring), not theme tokens. The tags use theme tokens only, so a pane whose hue equals a tag's hue still reads: the tag has its own outline, wash and text, and the contrast test runs against the row background, not against the pane's hue.
- Words carry the meaning. Each tag has visually hidden lead text, "Model " and "Seat " (the `ui-tag-sr` pattern), so a screen reader says "Model Sonnet 5.5" and "Seat Leader product". The tooltip is `Tag`'s `detail` (focus, hover and tap), which is also exposed as the description. The glyph is `aria-hidden`.
- Reduced motion: nothing animates. A model change replaces the tag; it does not flash (the summary line's flash is separate).

### 6.4 Legend

No legend in v1: the word is on every tag and the tooltip says what it is. If the operator wants a key after seeing it, a "Key" popover beside "Group by" listing only the families and roles present in the list is a small follow-up (D5).

## 7. Out of scope

- The data (#48221/#48222). Filtering the Swarm by model ("not Sonnet") is a natural follow-up on the filter bar, not part of this.
- Effort, route, account and context on the tag (all in the tooltip at most).
- A user setting for colours.

## 8. Test plan

Unit (vitest):
- `model-tag.test.ts`: the label shortening and id fallback table; family matching (Fable, Opus, Sonnet, Haiku, GPT/Codex, Gemini, route, local, unrecognised Claude); source and `pending_switch` states give solid, muted, arrow or none; unknown gives `null`.
- `seat-format` `seatHue` covers every role and an unknown one.
- Colour measurement per theme (6.3), including the selected-row surface.

Render (jsdom):
- A local row with a seat and a model shows both tags with the right hue class, the visually hidden lead text, and the tooltip; a known-model row has the tags line, an unknown one has none; a requested model is muted with the dashed class.
- A remote row shows its seat tag and no model tag (until P2).
- The seat badge slot rules still hold (`swarm-seats.render.test.tsx` and `swarm-row.layout.test.ts` pass unchanged apart from the class).
- My Agents tile and Atlas show the role colour, if D4 is yes.

Layout (jsdom, like `swarm-row.layout.test.ts`): at 360, 300, 240, 120 px pane widths, the header's drops are as before, the tags line is one line, never wraps, and is gone below 120 px; at chrome zoom 100%, 150% and 200% the tags line stays aligned with the name.

Look check (visual change rule), on the build branch rebuilt from current `origin/main`:
- macOS and Linux (the platform testers): a Swarm with agents of every model family and every role, a pane at about 300 px and at about 480 px, selected and unselected rows, a requested-only (muted) tag, the default dark theme, the light theme and high contrast, and chrome zoom 100%, 150%, 200%. Each answers pass or fail with a screenshot of anything wrong. The screenshots are kept for the PR body (not committed).
- The PR body records which platforms were checked on which commit.

## 9. Build plan (#48220)

1. **PR A (does not wait for #48222):** `Tag` `hue`, tokens and the colour test, `seatHue`, the seat badge as a coloured `Tag` (Swarm local and remote rows, My Agents, Atlas if D4), `model-tag.ts` and `SwarmModelTag` with fake data in tests and the tags line (empty until data arrives). Visual change: look checks on macOS and Linux.
2. **PR B (after #48222 P1 merges):** read `model_label`, `model_source`, `pending_switch` from the Swarm's runtime rows, show the tags line, look checks again with real data. After #48222 P2, remote rows follow with no frontend change beyond reading the same fields.

## 10. Decisions for the approver

D1. **Where the model tag goes.** Recommended: its own line under the header (3.1), at the cost of about 20 px per agent with a known model. Alternatives: (a) in the header after the seat badge, dropping at about 340 px, so at the usual 300 px pane it is not there (rejected: the main width loses the feature); (b) the recommended line, plus a Swarm menu choice "Show model tags" on/off kept in the pane's meta, default on.
D2. **A hue per Claude tier** (Fable, Opus, Sonnet, Haiku each its own colour), not one Claude hue with a lighter and darker step. Recommended, because per-tier questions are the point and the steps would sit under the 9-apart rule.
D3. **A marker in the row when an agent's model differs from its definition.** Recommended no (tooltip only); say yes if you want it visible.
D4. **Seat colour on My Agents and Atlas too.** Recommended yes (one `SeatTag`, small); no model tag there.
D5. **A legend.** Recommended not in v1.
