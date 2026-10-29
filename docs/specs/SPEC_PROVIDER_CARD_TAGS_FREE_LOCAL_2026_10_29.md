# SPEC: provider-card tags (Free, Local), card filters, and a free-signup guide

**Status:** proposed — approved by the operator on 2026-10-29 ("sounds good, go with recommendations"); nothing visible has shipped, the first PR adds only the access data (§5). Owner: Korp@narko. Written 2026-10-29 against origin/main `ff3eff050`.
Inputs: `docs/reports/REPORT_FREE_AND_LOCAL_PROVIDER_OPTIONS_2026_10_29.md` (every claim in the data file below comes from it, with its source and date) and Camper-2's `SPEC_UI_PANEL_PRIMITIVE_2026_10_29.md` §9 (the `Tag` primitive this builds on).
Mock (rendered from real tokens, not built): `docs/specs/mock-10964/cards.png`, source `cards.html` next to it.

## 1. The request

The operator, 2026-10-29 (quoted, relayed by AgentY and AgentX):

- "we want to introduce free options for beginning users. lets research the best free options, if signup is necessary, lets make the signup workflow easy, like a quick link and good instructions for afterward. Put a "Free" block, in blue, similar style to "Click to install" for options that are completely free, without requiring subscription."
- "we would also want to tag harnesses that can run local models. in that case, since there are so many tags, we may want to think of a better way to organize them. In this case, we'd have another tag for local"
- "those tags also need to be part of the greater framework" (so the tags are the kit's `Tag`, not picker CSS).

The research is done (report above). This spec answers: which tags a card can show and in what order, which providers get Free and Local, how a person filters, where the claims live so they go stale visibly, and what the signup step is.

## 2. Inventory: every tag, ribbon or badge a provider card or its neighbours show today

Read from the code on `ff3eff050`.

**On a template card (`AgentCard.tsx`, styles in `_picker.scss`)**

| What it says | Shown when | Where | Kind | File |
|---|---|---|---|---|
| "Click to install" / "Installing…" ribbon | CLI not found (`installed === false`) | bottom-right corner, solid `--success-color` fill, white `#fff` literal, `pointer-events: none` | an action (what the click will do) | `AgentCard.tsx:194`, `_picker.scss:321` |
| "Update available" badge (circle-up icon) | installed and `drift === "behind-pin"` | top-right, warning wash | a note on a card that works | `AgentCard.tsx:209`, `_picker.scss:350` |
| "+ New" pill | the agent has a current session, installed, not launching | bottom-right | an action | `AgentCard.tsx` (`agent-card-new-session-btn`) |
| launch spinner | launching | right | state | `AgentCard.tsx` (`agent-card-spinner`) |
| `agent-card--needs-install`, `--open`, `--disabled` classes | the same states | whole card | state styling, not tags | `AgentCard.tsx` |

The card shows no runtime badge on purpose (comment in `AgentCard.tsx`: a template is runtime-agnostic; Container or Native is chosen in the panel). `cli-catalog.ts` has a `popoverMarkdown` for an ⓘ popover, but `AgentCard.tsx` no longer renders one. The catalog's own header comment is stale on that point.

**On a "My agents" row (`MyAgentsList.tsx`)**

| What it says | Kind | Fate under this spec |
|---|---|---|
| `AgentRuntimeChip` (live runtime state) | status chip | unchanged here; moves to `Tag` in Camper's T3 |
| "no pane" tag (`agent-no-pane-tag`) | chip | T3 |
| active dot (`agent-active-badge`) | dot | `StatusDot`, already in the kit |
| NATIVE / CONTAINER `RuntimeBadge size="tag"` | chip (a property of a launched agent) | T3 |
| seat pill (`agent-seat-badge`, already a `Pill`) | chip | T3 |
| the bound account line, or "No auth" (`agent-recent-sessions-account`) | text line, not a tag | unchanged |
| vendor badge on the logo (`DualProviderLogo`) | logo overlay | unchanged |

Camper's §9.2 counts about 30 hand-rolled badge, chip and ribbon rules overall. This spec touches only the template card (T2 in their migration plan); My-agents rows, transcript chips and Kanban chips are theirs (T3 to T5).

## 3. The scheme: ribbon = action, chip = property, filter chip = narrowing

One rule, stated for the whole picker:

- **Ribbon: what clicking the card will do.** On a corner, outside the chip row, never counted by `TagList max`. Today's two: "Click to install" / "Installing…", and "+ New". They are exclusive today (`+ New` needs `installed !== false`) and share the bottom-right corner. Line-style (outlined), as the operator decided for the install ribbon on 2026-10-29. Not interactive on its own: the card is the button.
- **Chip: a property of the harness.** In a row under the caption. Today's "Update available" and the two new ones, Free and Local. A chip never does anything; its tooltip says why it is there.
- **Filter chip: a toggle that narrows the grid.** Above the template cards, §6.

**Tones** (Camper's mapping, from theme tokens only; no new token, no literal): Free `accent` (blue in the default theme, follows every theme), Local `neutral` with a house icon, Click to install `ok` ribbon with a download icon, Update available `warn` with the circle-up icon.

**Order** is the kit's fixed group order, never per call site: action (ribbon) > attention > plan (Free) > where (Local) > fact. Camper put "Update available" in `fact`, a quiet note on a card that works, which is what `_picker.scss` already says about it, so on a narrow card it collapses first (`+1`). I agree. A card therefore reads left to right: **Free, Local, Update available**, and "+N" cuts from the right.

**Decision for the operator (the one place I depart from the literal ask).** The operator asked for Free "similar style to Click to install", meaning a block on the card. The kit rule is ribbon = an action. Free is not an action, it is a property, and a blue Free ribbon next to a green "Click to install" ribbon on the same corner would be two ribbons fighting for one place (§8 describes the collision I saw in the first mock at 220 px, even with a chip). So Free is a **chip, same line style and the same family as the ribbon, accent blue**, sitting first in the chip row. It is as visible as the ribbon because it is the first chip and the only blue thing on the card. If the operator wants a literal corner block, the alternative is a top-left Free ribbon; I don't recommend it (it covers the logo).

**No Container/Native chip on template cards.** Camper's mapping lists Container and Native. On a template card they would be noise: `cli-catalog.ts` has 1 of 18 providers with `containerSupported: true`, so 17 cards would carry "Native only", and the runtime row in the new-agent form (merged today, #5830) already explains the difference at the moment of choice. Container and Native stay on launched agents (`RuntimeBadge`). If you want "Container-ready" as a positive tag later, it is one more `where` chip and one more filter; the scheme has room.

**Layout.** Cards that have chips or an action get a footer line: chips on the left, the corner action on the right, with space reserved so the ribbon no longer covers the caption (today it clips it: "Google's coding agent, with a Gemini API k…"). Under 300 px the action drops to its own line under the chips (container query on the card). The mock shows 220, 320 and 520 px, including the worst case: installed, a newer pinned CLI, a session in progress, Free and Local. A not-installed card shows the ribbon and no Update chip, as today. The cost is about 20 px of height on cards that carry chips; a card with no tags (Claude Code) is unchanged.

## 4. Which providers get which tags

Counts are for the 18 providers in `cli-catalog.ts`. **Mux Code is not in `cli-catalog.ts` and has no card today**; it would take Local as soon as it has one. Claims are from the report; "checked 2026-10-29" is the date on each.

### Free (7)

A provider gets Free only when its data entry has a `free` block, every claim in it has a source and a date, and the date is fresh (§5). Tooltip lines are generated from the data; the wording below is what the data says.

| Provider | Route | Tooltip (limits, then catches) |
|---|---|---|
| Antigravity | sign-in | Free with a Google sign-in, no card. A weekly quota; Google doesn't publish the number. Gemini Flash-class models; other vendors' models need a paid plan. |
| GitHub Copilot CLI | sign-in | Free with the Copilot Free plan (a GitHub account). A small AI-credit allowance, amount unpublished; auto model only. For individuals, not through an organization. |
| Grok Build | sign-in | Free to try with a grok.com or X account. Limits unpublished; a free Grok 4.5 promotion is "for a limited time". |
| OpenCode | sign-in (rotating) | Free models through OpenCode. No number published; users report hitting a limit. The lineup rotates, and some free models may use your data to improve the model. Works only inside OpenCode. |
| Kilo Code | sign-in | Free models, no card. 200 requests an hour per network address. The lineup changes, and some free models require data collection. |
| Cline | sign-in (rotating) | Free models with a Cline account. A rotating promotion with an unpublished quota; free usage may be used to improve models. |
| Gemini CLI | **key** | Free with an AI Studio key: 250 requests a day, Flash models only. Outside the EEA, Switzerland and the UK, Google may train on what you send. The old personal-account free tier moved to Antigravity on 2026-06-18. |

Every tooltip ends with a muted "checked 2026-10-29". Gemini CLI's card also names its successor (AgentX asked): the data entry has `successor: "antigravity"` and the card's caption/tooltip says "For a personal Google account, use Antigravity".

**Not Free, with the plain reason in a tooltip where it helps**

- Codex: **no Free chip.** The pricing page says ChatGPT Free "includes" Codex, but the CLI on a Free account is unconfirmed and the README points the other way. It gets a Local chip and one plain line: "Works with Ollama or LM Studio through the Responses API. No verified free cloud route." Stays open until someone tests a real Free account (§10).
- Claude Code: no free route from Anthropic. Kimi Code, Muse Code, Crush: paid. Qwen Code: its free tier ended 2026-04-15.
- Free software with a free *key* elsewhere (Aider, goose, Pi, OpenClaw, Dsh, Qwen Code, Crush): no chip, so the blue Free stays honest. Their cards can mention a free key in the signup guide only if a provider opts in (the data file has room: a `freeKey` entry), and v1 does not.

### Local (14 of 18 cards; Mux Code when it has one)

Local means the CLI documents a way to use a model on this computer. It does **not** say the quality is good; the tooltip says what is known. Each line is generated from the data (`local.how`, `local.note`).

| Provider | Tooltip |
|---|---|
| Codex | Works with Ollama or LM Studio through the Responses API. No verified free cloud route. Rough fit: open issues with MCP tools on local servers. |
| Claude Code | Works through Ollama's or LM Studio's Anthropic endpoint. Anthropic doesn't support it; expect rough edges. |
| Qwen Code, OpenCode, Kilo Code, Pi, Grok Build, goose, Crush, Cline | Point it at an OpenAI-compatible server (Ollama at `:11434`, LM Studio at `:1234`). Needs a model that supports tool calls. |
| GitHub Copilot CLI | Finds Ollama on this computer by itself. Needs a model with tool calls and streaming. |
| OpenClaw | Use Ollama's native API, not the `/v1` URL (its docs say `/v1` breaks tool calling). |
| Aider | Works with Ollama or LM Studio. Edits as text, so small models do better here than in tool-calling agents. The project has had no release since 2025-08. |
| Dsh | `ollama launch dsh`; a developer preview. |

**No Local chip (claims not met):** Gemini CLI (no local support), Antigravity (no BYOK or custom endpoint), Muse Code (none found), and **Kimi Code, whose local support is unverified** (a fetch summary only), so it waits for a check. Local on 14 cards, per the report's table, minus Kimi.

**Before the Local chips ship**, I'd like to run each of the 14 against one local model (`qwen3-coder:30b`) on a Linux or Windows test host and record pass or fail in the data file as `local.testedOn`. The chip would then say "tested by us". That is a follow-up task (§10), not a blocker: until it exists the tooltip says "documented, not tested by AgentMux".

## 5. The data file: claims with a date, so they go visibly stale

**Where:** `frontend/app/view/agent/defaults/provider-access.ts`, a typed const next to `cli-catalog.ts`, keyed by provider. One file, reviewed like code, with a test. (Not JSON: the type enforces that a claim has a source and a date.)

```ts
export interface AccessClaim {
    text: string;          // one short sentence, in the words the tooltip shows
    source: string;        // the page it was read on (https)
    verifiedOn: string;    // ISO date, the day someone read that page
}
export type Tri = "no" | "yes" | "unknown";

export interface ProviderAccess {
    provider: string;                     // = cli-catalog provider
    free?: {
        route: "signin" | "key";          // how a person gets in
        lead: string;                     // the tooltip's first line, e.g. "Free with a Google sign-in"
        limits: AccessClaim;              // quota, models, region
        catches: AccessClaim[];           // training on data, rotation, promo end (at most 2 shown)
        card: Tri; phone: Tri;            // "unknown" is allowed and is shown as "not stated"
        signupUrl: string;                // THE one link: the sign-up or key page
        steps: string[];                  // 2 to 3 short lines for after the link (§7)
        expiresOn?: string;               // a promotion with a known end: after it, no Free chip
    };
    local?: {
        how: AccessClaim;                 // flag, config or env var, as the CLI's own docs say
        note?: AccessClaim;               // the catch, e.g. "Anthropic doesn't support it"
        testedOn?: { model: string; date: string; ok: boolean };   // the follow-up in §4
    };
    successor?: { provider: string; why: string };   // Gemini CLI -> Antigravity
    plain?: AccessClaim;                  // a line for a card with no tag, e.g. Codex's
}
```

**Freshness rules (computed from `verifiedOn` at render, no timers):**

- Within 30 days: the tooltip ends with a muted "checked 2026-10-29".
- 31 to 90 days: the same line turns to warn and says "checked 41 days ago, may have changed".
- More than 90 days, or past `expiresOn`: the Free chip turns **neutral** and its tooltip opens with "Not re-checked since 2026-07-01", always showing the date so it never reads as a bug. It is not hidden (a vanished tag looks like a bug), but it stops being blue, the colour that means "we checked". A Local claim is treated the same way.
- Dates take the oldest claim in the entry.

**Keeping it true:** a Schedule task (monthly) "Re-verify provider access" runs the same six research prompts and edits this file; the report's method section is the checklist. A freshness check lists entries older than 90 days and **reports them to a leader**; it is not a CI job that fails a PR (AgentX: a PR must not go red because a calendar moved). Every entry also gets a unit test that its claims carry an https source and a date in the past.

**Why a data file and not strings in `AgentCard`:** the Gemini story already flipped once (2026-06-18) while blogs and a vendor table still show the old facts. The people reading this are beginners; a wrong "Free" costs them time and trust.

## 6. Filter chips above the card grid

In the "New agent" section, under its intro and above the cards: **Free 7**, **Local 14**, **Installed n**. Camper's `FilterChips` (multi-select toggles, live count).

- Selecting more than one **narrows** (AND): Free + Local shows providers that are both (OpenCode, Kilo, Cline, Copilot, Grok at today's data). Installed means `installed === true`; the count appears once the install probes have answered, never a wrong zero.
- Counts are live: each chip's count is what selecting it would leave, given the others.
- Applies to template cards only; the My-agents list keeps its name filter and sort.
- Not persisted. A hidden-by-default filter is how a card "disappears"; the chips are visible and a cleared state is the honest default. (The runtime choice persists because it is an input; a filter is a view.)
- Empty result: one line, "No agent is Free, Local and installed.", and a "Clear filters" button.
- Keyboard and screen reader: chips are buttons in tab order after the section intro, `aria-pressed`, and a polite live region says "7 of 18 shown".
- A card hidden by a filter is not in the tab order; the picker's default-focus card logic keeps working if the default card is hidden (focus falls to the first shown).

## 7. The signup flow: one link, then clear next steps

Goal (the operator's words): "a quick link and good instructions for afterward". The flow lives in the card's docked panel (`AgentCreatePanel`, the P1 "details below the card" pattern, built on Camper's `Panel` when it lands). The panel's steps today are tools, install and create (`Step = "tools" | "install" | "create"`). For a provider with a `free` block and **no usable sign-in or key yet**, a "Get started free" block is added after install and before create. For someone already signed in it does not appear.

Two routes, from the data's `route`:

**A. Sign-in (Antigravity, Copilot, Grok, OpenCode, Kilo, Cline).**
1. One button, "Open sign-in", opens `signupUrl` in the **system browser** (`getApi().openExternal`, as the panel already does for docs). Not an in-app browser pane: Google and others refuse sign-in in embedded browsers, and the person's password manager lives in their own browser.
2. "Come back here." AgentMux watches the existing auth check (`authCheckCommand`, the same one that gates launch today) and the step ticks off when it passes. For an `oauth` provider the existing in-app login panel (`InAppLoginPanel`) takes over for the code paste; for `cli-managed` (Antigravity) the existing "use terminal" path.
3. "Name your agent and start." The note under the steps is the card's limits line from the data ("Weekly quota; /usage shows it").

**B. Key (Gemini CLI).**
1. "Open Google AI Studio" opens `signupUrl` in the system browser.
2. A masked field in the panel. **The key never goes in the chat and never into an environment line the person can see.** It is stored through the account-key path the form already has (`AccountKeyVerifyCommand`: the OS keychain, one user-initiated live check before storing, a masked tail kept, never read back to the UI), bound to the new agent. Launch already reads the bound account's key for `authType: "api-key"`, so nothing new is needed on launch for Gemini CLI.
3. Start. The limits and the training catch show once, under the field, in the person's words ("free-tier data may train Google's models outside the EEA").

**What AgentMux writes for a key-based option.** Gemini CLI needs only the account. The Codex-with-another-key case from the report (`model_providers` in `CODEX_HOME/config.toml`, the key read from the variable named in `env_key`, `wire_api = "responses"`) is not part of this build: no free cloud key is verified to work with Codex. It belongs to the Local/model-source spec (§10), where AgentMux writes that file.

**Where the key lives (decided 2026-10-29 by AgentY and AgentX).** The bound account in the OS keychain, entered through the masked-field pattern, so a key typed here behaves exactly like one entered in Connectors or Accounts. The rule from AgentY: a key is never typed into or shown in a chat or transcript, and never goes into a container. (The Vault pane is a separate store that agents request from; it is not used here.)

**Account providers.** The identity form has `google`, `openai`, `anthropic` and `custom`. Gemini uses `google`. OpenRouter or Groq keys (needed by the later free-key flow for the BYOK tools) would be `custom` or two new providers; not needed for v1.

**What the panel never does:** pay for anything, create the account on the person's behalf, or open a link that is not in the data file (the URL comes from the reviewed file, https only).

## 8. Mocks

`docs/specs/mock-10964/cards.png` (source `cards.html`): the filter chips; five cards at 220, 320 and 520 px (OpenCode not installed with Free + Local and the install ribbon; Gemini installed with Free and an update; Kilo installed with Free, Local, update and a session; Codex with Local only; Claude with none); the three tooltips (Antigravity's Free, Gemini CLI's Free with the catch and the successor line, Codex's Local with the plain line); and the signup panel for both routes.

What the mock showed me, and the spec now says: at 220 px the install ribbon and the chip row collided (the Local chip was hidden under the ribbon), so under 300 px the action drops to its own line; and at 220 px a "+1" chip next to a "+ New" pill read as "+1 + New", for the same reason.

## 9. Built on the kit, no picker-only CSS

Everything visual here is `Tag`, `TagList`, `FilterChip(s)` and `Panel` from Camper's spec: no new tag SCSS in `_picker.scss`, and the card's existing `agent-card-install-ribbon` and `agent-card-upgrade-badge` rules are **deleted** in the PR that moves the card to `Tag` (their white-on-green text fails contrast on 11 of 13 themes today). What this spec adds beyond the kit:

- The footer layout of the card (chips left, action right; the container query under 300 px). That is the card's own layout, not a tag style.
- The data file, the freshness rule, and the filter state.

Agreed with Camper-2: `detail` is a JSX body that wraps at the 240 px popover width and holds about four short lines; a tag and its tooltip never hold a link (sign-up lives in the panel); the ribbon sits outside `max`; Upgrade is `fact`; `FilterChip` has a live count. If `TagList` needs a `where` group member for Local or a preset, that is theirs to add in T1.

## 10. Build plan

Camper-2's **T1 (the `Tag` primitive) lands first**. Then, in order, each PR with before and after screenshots of the card at the three widths:

1. **P1: the data file and the card tags.** `provider-access.ts` and its tests, the card on `Tag` (Free, Local, Update available, ribbon, "+ New"), the footer layout, the tooltips, the freshness rule, the freshness report to a leader. Deletes the two old rules. (This is Camper's T2.)
2. **P2: the filter chips** in `AgentPicker`: Free, Local, Installed, empty state, the live region, tests.
3. **P3: the "Get started free" step** in `AgentCreatePanel`: both routes, the auth-check watcher, the masked key field and its account binding for Gemini CLI, tests. Needs the §7 decision on the Vault.

**Follow-ups, not in this build (each a task):**

- **Codex on a ChatGPT Free account.** Someone with a real Free account runs `codex` and records the answer; until then it has no Free chip. (AgentX asked for this to be listed for the operator, not claimed.)
- **Local test matrix:** the 14 Local CLIs against one local model, on a Linux test host, results into `local.testedOn`.
- **Local model source** (its own spec): detect Ollama and LM Studio (Ollama is already an optional toolchain entry, `toolchain-catalog.ts`; the operator's machine serves it on `:11434` with no model pulled), a "Model source" choice in the create form, and the per-CLI writers (`config.toml` for Codex, Kimi and Grok; JSON for OpenCode, Kilo, Pi, Cline, goose, Crush, Copilot and Qwen; env for Claude, Aider and goose). The report's table is its input.
- **Monthly "Re-verify provider access"** Schedule task.
- **Mux Code card**, so it can take Local.

## 11. Decisions for the operator (recommendation first)

1. **Free as a chip, not a second ribbon** (§3). Recommended. The alternative is a literal top-left corner block.
2. **Seven Free chips, no Free chip for Codex** until it is tested on a real Free account (§4). Recommended.
3. **A stale claim goes neutral, not hidden** (§5). Recommended.
4. **No Container/Native chip on template cards** (§3). Recommended.
5. **Local chip on Claude Code and Codex, with a plain caution in the tooltip** (§4): they work, but Anthropic doesn't support Claude Code on non-Claude models and Codex is the roughest fit. Recommended (AgentX asked for Codex's). The alternative is to omit both until tested.
6. **The key goes through the account store the launch already reads, with a masked field** (§7). **Decided by AgentY and AgentX on 2026-10-29**, not left open: never in a chat or transcript, never into a container.
7. **System browser for sign-up, not an in-app pane** (§7). Recommended.

## 12. What this spec could not verify

Everything marked unverified in the report carries over: Codex CLI on a Free account; card and phone requirements for most free routes (so each `card`/`phone` is `unknown` and the tooltip says "not stated", never "no card" unless the vendor's page says it, as Kilo's does); the size of Antigravity, Copilot, Grok, Cline and OpenCode quotas; whether OpenCode free models work signed out on v2; Kimi's local support. The Local chips are documentation-level claims until the test matrix runs. No provider was used with a real account to write this.
