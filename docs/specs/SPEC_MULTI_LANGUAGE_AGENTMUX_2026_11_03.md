# Spec: Multi-language AgentMux, for an international audience

**Date:** 2026-11-03 · **Task:** #21623 (spec), #21624 (build) · **Author:** Clamk@narko

**Status:** active: approved by the operator 2026-11-04, build #21624 under way; progress in section 13
**Builds on:** `docs/analysis/ANALYSIS_I18N_INTERNATIONALIZATION_SCOPING_AND_COST_2026_09_07.md` (AgentX), re-checked against `origin/main` at `623bf54b2` (2026-11-03).
**Reads:** AgentX owns product direction and should read this before approval.

## 1. The decision, and the recommendation

The operator wants an international audience: a person in their own language should be able to install, sign in, use agents and read the docs without English.

**Recommendation.** Ship in three waves, English staying the source language:

| Wave | Languages | Why (data in section 4) |
|---|---|---|
| **1** (first milestone) | Japanese `ja`, Simplified Chinese `zh-Hans`, Brazilian Portuguese `pt-BR` | Largest developer populations where English is the biggest hurdle, and the fastest-growing (Brazil +3.3M, Japan +2.6M developers in a year); Japan is a top-five country for Claude.ai use; China is where Kimi and Qwen, two CLIs we host, are home |
| **2** | Korean `ko`, Spanish `es`, German `de`, French `fr` | Korea is top-five for Claude.ai use; `de` and `fr` are large and rank high in developer surveys but read English well, so they gain less per translated word; Spanish has no country in the published top ten, so its place is a data gap to close (4.3) |
| **3** | One right-to-left language (`ar` as the test), then Indonesian, Traditional Chinese, others on demand | The app is not RTL-ready (about 340 physical left/right style declarations against 5 logical ones); do the groundwork in wave 1 and 2, ship RTL when it is cheap |

**Architecture in one paragraph.** One flat-JSON dictionary per locale, loaded on demand as a lazy `import()` chunk (this works today: the 2026-09 blocker is gone, 3.1). `@solid-primitives/i18n` for lookups, native `Intl` for dates, numbers, plurals and relative times, no new formatting library. The same JSON files feed the native Rust strings (menus, tray, fatal dialogs): one format, one translation pipeline, no Fluent. Errors are split by audience: text the agent reads stays English; text the person reads is coded and translated; legacy raw strings show a translated frame around English details. **Agent reply language is its own setting**, default "follow the person's message", mapped per provider (section 3).

**Milestone 1, worth shipping on its own:** the app in `ja`, `zh-Hans`, `pt-BR` for the first-hour flows (install, first run, sign-in, open an agent, panes and tabs, settings), native menus and dialogs, the 100 most common errors, the landing site in the same three languages with correct `hreflang`, the store listings, the reply-language setting, and the "getting started" docs. Everything else falls back to English per key and the language is labelled **Beta** until it passes the coverage gate (8.2). Reviewers are the main cost and the platform can cost nothing (7.2); the budget figures are in the private record.

**Decisions the operator must make** (details in section 11): (1) wave 1 languages; (2) docs scope for milestone 1; (3) paid native reviewers; (4) apply to Crowdin's free open-source plan; (5) ship under-covered languages as Beta with per-key English fallback; (6) RTL after wave 2.

**Fix now, regardless of this spec** (section 9): two hardcoded `"en-US"` date formats and four hand-built "5 min ago" strings, as one small bug PR.

## 2. What the 2026-09-07 analysis got, re-checked against main

| Finding then | Today | Effect on the plan |
|---|---|---|
| SolidJS frontend, no i18n library | Same (`solid-js` 1.9, no i18n dependency in `package.json`, `Cargo.toml` or `frontend/`) | Retrofit, unchanged |
| 1,500 to 3,000 frontend strings | Direct counts now: 589 `title`/`aria-label`/`placeholder`/`label` string attributes (264 then), about 440 JSX text nodes (about 310), 56 `throw new Error("…")` (100), 19 literal toast messages (59), plus template strings and the catalogs. Counting methods differ from the analysis, so treat the trend, not the digits | Revised estimate **2,500 to 4,500 strings**; Phase 0 replaces it with a measured count |
| **Single bundle: `manualChunks` is off because of the old WebKitGTK host** | The comment is stale. The host is CEF on all three platforms (no WebKitGTK dependency in the tree), and a build from 2026-10-14 in this clone has **164 separate lazy chunk files** (the shiki and mermaid grammars load this way) | **Blocker removed.** Per-locale `import()` works with no build change. Phase 0 re-confirms it from the CEF scheme on Windows, macOS and Linux |
| 456 backend error sites, 3 structured | `AgentMuxError::` is used 7 times; 1,404 function signatures return `Result<_, String>`; 16 `AmxCode` variants, one of them `Legacy`, which the frontend echoes verbatim | Same shape, bigger. Converting sites is the wrong plan; split by audience instead (6.7) |
| 22 `toLocale*` calls, 0 `Intl` | **61 `toLocale*` calls in 43 files, 1 `Intl` use.** The two `"en-US"` sites moved: `frontend/app/store/app-update.ts:79`, `frontend/app/view/agent/components/InputHistorySearch.tsx:29`. Relative time is hand-built in four places (`notification/usenotification.tsx`, `statusbar/HistoryUsagePopover.tsx`, `statusbar/lan-peer.ts`, `devtools/diag-panel.tsx`) | A formatting helper is a prerequisite, and the bug PR (section 9) |
| RTL: 29 scss files, 57 physical declarations | 215 scss files; 338 physical `left/right` declarations (a broader pattern than then) against 5 logical ones | RTL is a project of its own: wave 3 |
| 50 to 70 native strings in about 6 files | Same order: `macos_menu.rs`, the launcher's supervisor, memory and upgrade dialogs, the CEF init and Linux sandbox dialogs | Small count, three dialog APIs |
| Surfaces: the app only | Not covered: landing site, docs, stores and installer, release notes, the mobile app, the cloud console, **what agents reply in** | Added below (section 5) |

## 3. Research: how the providers AgentMux runs handle language

Desk research on 2026-11-03: official docs, changelogs and GitHub issues, cited in section 12. Where a fact comes from a search summary rather than the page, the research notes mark it; claims I could not verify are listed in 12.2.

**The pattern in one line:** every CLI we host ships its terminal interface in English only. The vendors that localise do it in their apps (Claude apps, ChatGPT, Gemini, Kimi apps, Codex desktop, VS Code, Cursor's editor), and the model's reply language is a separate thing from the interface language.

| Product | UI languages and how a person picks | Reply language | CLI text localised? | Translation approach |
|---|---|---|---|---|
| **Claude Code** | CLI English only; every localisation request closed as duplicate or not planned (#29147, #50680, #64472, #72257). Claude apps: Help Center lists 11, a 2026-10-29 report says 33; pick in profile menu | **`language` key** in `settings.json` (any scope), free string passed verbatim to the model; also sets the dictation language and session titles. Unset: follows the message | No | Not documented |
| **Codex CLI / ChatGPT** | CLI English only ("no plans to provide localization for the TUI", #26136). Desktop app: bundled translations, `[desktop] localeOverride` or `ELECTRON_LOCALE_OVERRIDE`, but gated by a remote flag `enable_i18n` and the MSIX declares only `en-US` (#24741, #51926). ChatGPT: about 58 languages, auto-detect plus Settings | No dedicated key. `-c developer_instructions=…` or an `AGENTS.md` line. `/review` ignored it (#5113) | No | react-intl-style bundles in the desktop app; process not public |
| **Gemini CLI / Gemini app** | CLI English only; the i18n epic (#17814) and "reply in the user's language" (#1871) closed as not planned. App: 70+ languages from the browser or device | No key; a `GEMINI.md` line is the one lever | No | None for the CLI |
| **Qwen Code** | **Best model.** 8 built-in UI languages, `general.language` setting, `QWEN_CODE_LANG` env, `LANG` fallback, then Intl, drop-in packs in `~/.qwen/locales` | **Separate** `general.outputLanguage`, with `auto` meaning "follow the user's message". An early version hard-locked "you MUST respond in X" from the OS locale and caused bugs (#2830, #6943, fixed in #6953) | Yes | Plain JS key-value files keyed by English text; zh-TW vocabulary enforced in CI; docs machine-translated |
| **Kimi** | CLI English only (#3571, #4607 open); desktop asks for a language on first launch; apps list about 13 languages | No key. Follows the user; the 2026-10 changelog extends "same language" to its reasoning; reasoning still reportedly English for Chinese prompts (#4458) | No | None public |
| **Copilot CLI / Copilot in VS Code** | CLI English only (#3681 asks for `COPILOT_LOCALE`, no answer). Chat follows VS Code's display language | VS Code: `github.copilot.chat.localeOverride` (default `auto`). CLI: none | CLI no | VS Code pipeline |
| **VS Code** | Language packs; `--locale=<code>` or `locale` in `argv.json`; about 14 locales; **a missing pack silently shows English**; `argv.json` handling is fragile (#151200, #174848) | n/a | Yes | Microsoft Localization Platform plus a community project; extensions use `l10n.t()` with per-key English fallback and pseudo-localisation in `@vscode/l10n-dev` |
| **Cursor** | Same packs as VS Code, but Cursor's own Settings, Agent and account screens "stay in English" (staff, 2026-07) | The old language setting was folded into User Rules; staff: language comes "from the context of your conversation and code plus your User Rules" | Partly | VS Code packs |
| **Aider** | No UI localisation | **`AIDER_CHAT_LANGUAGE`** and a separate **`AIDER_COMMIT_LANGUAGE`** | No | n/a |
| **Cline** | Extension translation PR unmerged (July 2026) | "Preferred Language" in its settings (not a file a host can write); commit messages ignored it (#7055) | No | n/a |
| **OpenCode, Crush, Goose, Pi, Grok, Antigravity, Muse** | Terminal UIs English or unknown; Crush closed its request as "very unlikely"; Goose declined | Crush and others: steer through their instructions file | No / unknown | n/a |
| **Kilo, OpenClaw** | Extension and web UIs localised (22 and 20+ locales) | Kilo: follows the UI; OpenClaw: none found | CLI unknown | OpenClaw translates its docs into about 18 languages |

### 3.1 What AgentMux should copy

1. **Two settings, never one** (Qwen, Kimi apps, Cline, Cursor staff): `app:locale` for the AgentMux interface and `agent:reply_language` for what agents answer in. A developer on a Chinese-language OS who wants English replies is common (Qwen #6943).
2. **Reply language defaults to "follow the person's message"** (Qwen `auto`, Cursor, Kimi, Copilot). A fixed language is an opt-in. Qwen's hard lock from the OS locale was a bug.
3. **UI language defaults to the OS locale with an explicit override and the effective value shown** (ChatGPT, Gemini, Qwen). Never gate shipped translations on a remote flag, and never show a silent English fallback with no explanation (Codex `enable_i18n`, VS Code missing pack).
4. **Per-key fallback to English, a stable BCP 47 code, pseudo-localisation** (VS Code `l10n`). Keys are semantic ids, not English text (Qwen's English-as-key files break when the English is edited).
5. **Keep commit-message and code-comment language separate** and defaulting to English or "follow the repository" (Aider's `AIDER_COMMIT_LANGUAGE` is the clean model; Copilot, Cline and Cursor all leaked chat language into commits).
6. **Say the scope in the instruction** the vendors do not document: "Reply in Japanese. Keep code, identifiers, commit messages and tool-call arguments in English unless I say otherwise" (drift to English after compaction and from English git output is the top complaint: Claude Code #59299, #99800).
7. **Drop-in language packs for the community** (Qwen's `~/.qwen/locales`, VS Code's community project): volunteers are waiting in every tracker (Kimi #4607, OpenCode #37216, Cline #12518). Build the format so a pack is one folder (section 6.2), and add the review step before it ships.
8. **Terminology discipline for CJK**: Simplified and Traditional Chinese, and `pt-BR` and `pt-PT`, are separate locales with a glossary (Qwen enforces zh-TW vocabulary in CI).

### 3.2 Where AgentMux must pass the person's language through to the provider

AgentMux already writes each provider's startup instructions file at launch (`build_config_files` in `crates/srv/src/backend/agent_config.rs`, filename from `startup_instructions_filename`, `{{VARIABLE}}` templating) and passes `LANG` and `LC_ALL` to agents (`agent_env.rs`). So there is one universal lever and a few native ones:

| Provider | Reply language lever (use this) | Notes |
|---|---|---|
| Claude Code | Launch flag `--settings '{"language":"<english name>"}'` (ranks above user and project files, below managed). Also a language block in the generated `CLAUDE.md` | Use the English name (`japanese`): the value is passed verbatim. Do not edit the user's `~/.claude/settings.json` |
| Codex | `-c developer_instructions="…"` at launch, plus a block in the generated `AGENTS.md` | `/review` ignored `AGENTS.md` language in 0.46 (#5113): test before promising |
| Gemini | A block in the generated `GEMINI.md` | The only documented lever |
| Qwen Code | Settings key `general.outputLanguage` (and `QWEN_CODE_LANG` for its own UI, only if the person's UI language is set) | Prefer the settings key over its `output-language.md` file (older builds regenerated it every launch) |
| Aider | `AIDER_CHAT_LANGUAGE`, and `AIDER_COMMIT_LANGUAGE` if the commit setting is on | Native env vars |
| Copilot CLI, Kimi, Crush, Cline, OpenCode, OpenClaw, Goose, Grok, Pi, Kilo (CLI), Antigravity, Muse, Muxcode | A block in the generated instructions file | Kimi now documents `AGENTS.md` (`~/.kimi-code/AGENTS.md`); our catalog says Kimi has no native file. **Re-verify** before relying on it. Cline's "Preferred Language" is not a file we can write |
| UI language of the CLI itself | Pass through only where documented: Qwen's `QWEN_CODE_LANG`. Do **not** set `LANG` to carry a language (only Qwen reads it); keep a UTF-8 `LANG` so CJK renders | Show in the provider's settings page whether it honours the language or only gets a prompt hint |

Rules for the instruction block: AgentMux owns a marked block (start and end comment markers) inside the instructions file and never rewrites the person's text; it is written once per change, not rewritten at every launch (Qwen's per-launch regeneration overwrote user edits, #2830); it repeats in project instruction files because those are re-read on every run, which is the mitigation for drift after compaction.

### 3.3 What they do badly, to avoid

- English-only terminal UIs with no plan (Claude Code, Codex, Gemini, Copilot, Kimi): AgentMux can translate only its own chrome around a pane. Be honest in the UI about it.
- Translations that ship hidden or silently fall back (Codex, VS Code).
- A locale override that is silently ignored, or does not persist (Codex `localeOverride`, Claude `/config`): show the effective value.
- IME and RTL treated as extras: Claude Code has many open composition and reversed-text bugs (#36500, #48257, #89489). They are release gates here (6.8).
- Declaring one locale in the installer (Codex's MSIX declares only `en-US`, #51926).

## 4. Languages and the data behind them

### 4.1 The data

- **Developers by country, GitHub Octoverse 2026** (September 2026): United States 38.3M, India 32.0M, **China 12.4M, Brazil 10.2M, Japan 7.15M**, United Kingdom 6.47M, Indonesia 6.34M, **Germany 6.30M**, Russia 5.07M, France 4.75M. Growth in a year: India +10M, the US +10M, **Brazil +3.28M** (third most), **Japan +2.6M**. Top countries by pushes to public repositories: US, India, China, Brazil, Germany. (GitHub's own platform data, by stated location.)
- **Stack Overflow Developer Survey 2026** (30,903 responses): top countries US, **Germany (9.3%)**, UK, India, Canada, **France**, Netherlands, Poland, Australia, Ukraine.
- **Anthropic Economic Index** (March 2026 report, Claude.ai traffic): the countries with the most total use are the US, India, **Japan**, the UK and **South Korea**. No Claude Code breakdown is published. Per person, small technology-heavy countries lead.
- **Our own hosted CLIs** include Kimi and Qwen, the home-market tools for Chinese developers, which is a reason for `zh-Hans` the other vendors' data does not capture.

### 4.2 The choices

| Language | Wave | Reason | Caveat |
|---|---|---|---|
| Japanese | 1 | 7.15M developers (+2.6M), top-three Claude.ai country, high expectation of localised software | Needs CJK font stack and `lang="ja"` (Kimi shipped Japanese with Chinese glyph variants, #2575) |
| Simplified Chinese | 1 | 12.4M developers; Kimi and Qwen users; Qwen already ships `zh-CN` UI strings we can learn from | Anthropic and OpenAI services are restricted in China; the audience is those who reach them, plus domestic CLIs. Keep `zh-Hans` and `zh-Hant` separate |
| Portuguese (Brazil) | 1 | 10.2M developers, fastest growing large market (secondary coverage reports a GitHub projection of it passing China by 2030; not on the Octoverse page itself) | Separate from `pt-PT` |
| Korean | 2 | Top-five Claude.ai country, top-ten pushes (secondary source) | Hangul IME bugs seen in hosted CLIs: test composition |
| Spanish | 2 | Largest remaining language community by speakers | **Data gap:** no Spanish-speaking country is in the published top ten, so we cannot size it from these sources; close the gap with landing-page country logs (4.3) and choose `es-419` or `es-ES` as the base with a regional pass |
| German | 2 | 6.30M developers, #2 in the Stack Overflow sample, strong enterprise and privacy buyers | Reads English well: lower marginal gain, higher text expansion (+30%) to test |
| French | 2 | 4.75M developers, top-ten in both surveys | Same as German |
| Arabic (RTL test) | 3 | Largest right-to-left language | No country data in the sources above; Hebrew is the alternative RTL test with a smaller audience but a dense developer base |
| Indonesian, Russian, Hindi | on demand | Indonesia 6.34M, Russia 5.07M, India 32M | Indonesian and Hindi developers mostly work in English tools; Russia has payment and sanctions constraints for stores |

### 4.3 Close the data gap in Phase 0

Pull the country and `Accept-Language` mix from the landing site's access logs and the download endpoints, add it to this table, and reorder wave 2 if it disagrees. No telemetry is added to the app for this.

## 5. Surfaces, in ship order

| # | Surface | Today | Plan |
|---|---|---|---|
| 1 | **App UI**: frontend strings, native menus and dialogs, errors | English; no framework | Sections 6 and 8. Native strings (about 70): macOS menu, tray, launcher fatal and upgrade dialogs, CEF init and Linux sandbox dialogs, splash |
| 2 | **Agent replies** | Follow the message; no setting | `agent:reply_language`, per-provider mapping (3.2). Ships with the app milestone |
| 3 | **Landing** `agentmux.ai` (SolidJS, prerendered; its own private repository) | `<html lang="en">`, no `hreflang`, a handful of URLs in the sitemap, about 3,000 words of copy | Locale routes `/ja/`, `/zh-hans/`, `/pt-br/` prerendered per locale, `hreflang` and `x-default` in the page and the sitemap, `og:locale`, a language switcher that remembers the choice, no automatic redirect by IP (search engines and VPN users). Reuse the app's dictionary format |
| 4 | **Store listings, installer, release notes** | MSIX manifest declares `<Resource Language="en-us"/>` only; winget manifest English; releases are English changesets | MSIX `Resources` list each locale (or the Store ignores the language, Codex #51926); Microsoft Store listing per language; winget `*.locale.<tag>.yaml` description manifests; macOS `CFBundleLocalizations` and `.lproj` for system dialogs; Linux `.desktop` `Name[ja]=` and AppStream `xml:lang`; App Store and TestFlight metadata per locale for the mobile app; Homebrew has no localisation. Release notes: the top three to five highlights per release translated; the full changelog stays English (changesets are developer-voiced) |
| 5 | **Docs** `docs.agentmux.ai` (`agentmux-docs`: Astro Starlight 0.42, 35 content pages, about 108k words including code) | English; no locales configured | Starlight has built-in i18n: configure `locales`, per-page fallback to English with its notice, and multilingual search. Translate the **getting-started set first** (about 8 pages, about 15k words, chosen in Phase 0); other pages stay English with the notice. Code blocks, commands and API names are never translated |
| 6 | **Cloud console** (React + Cloudscape; its own private repository) | English | Cloudscape ships message bundles for many locales and an `I18nProvider`; wave 2, after the app |
| 7 | **Mobile app** (`agentmux-mobile`: Flutter) | English | Flutter's own `gen-l10n` with ARB files; same glossary and translation memory; wave 2 |

## 6. Architecture, re-checked today

### 6.1 Library and bundle

- **Frontend: no library (decided in Phase 1).** `@solid-primitives/i18n` v2 was the plan: about 0.6 kB, dictionary-based, reactive. What this app needs on top of it (suffix plurals by `Intl.PluralRules`, a per-key fall back to English, a loaded-state, a per-locale lazy file) is the larger part, so the translator is about 150 lines in `frontend/app/i18n/` over `createSignal`, with the same flat-dictionary shape; adopting the library later would change only the lookup. Alternatives considered: `@lingui/solid` (macro extraction, newer), Paraglide, Intlayer (its comparison articles are vendor-authored). Revisit if extraction tooling matters more than size.
- **Per-locale chunk** `locales/<tag>.json`, lazy. A small `core` namespace (shell, first run, sign-in, settings) is requested first so the first paint is translated; area namespaces load with their views.
- **Size:** about 25 to 70 KB gzip per locale (the analysis' estimate, unmeasured); only the active locale loads. Not a constraint against a 431 MB portable build.

### 6.2 Key convention and packs

- Semantic dot-path ids by area: `agent.composer.placeholder`, `settings.language.label`, `errors.AMX-PATH-NOT-FOUND.title`. English is the source file (`locales/en.json`); a `_context` note beside any key that is ambiguous, and screenshots in the translation platform.
- Plurals use i18next-style suffixes (`.one`, `.other`, plus `.few`, `.many` where a language needs them) resolved with `Intl.PluralRules`; interpolation is `{{name}}`. No ICU message parser ships. Gender and select are written around in the copy.
- A **pack** is one folder `locales/<tag>/` plus a manifest (tag, name in its own language, base locale for fallback, reviewer, coverage). A user-level override folder (as Qwen's) is allowed after wave 1 for community packs, which load under a "community" label until reviewed.

### 6.3 Locale choice

- `app:locale` in `settings.json`: `"system"` (default) or a BCP 47 tag. Negotiation: exact match, then language match within the supported set (`pt-PT` gets `pt-BR`; `zh-TW` does **not** get Simplified silently, it gets English until `zh-Hant` exists), then English.
- The picker lists each language in its own name, shows the **effective** locale and the coverage of the chosen one, and applies live. Native menus switch on next launch first, live later.
- `<html lang>` follows the locale, with per-locale font stacks (`:lang(ja)` before `:lang(zh)`).

### 6.4 Formatting

The formatters live in `frontend/util/format-time.ts` (the shared time module; `format-bytes.ts` and `format-count.ts` sit beside it): dates, times, relative times, numbers, bytes, lists, on `Intl` with the app locale (each takes an optional `locale`, the browser's when left out; the app's `appLocale()` is passed once the picker ships); they replace the 61 `toLocale*` calls and the four hand-built relative times. A lint rule forbids new raw `toLocale*` calls and literal `"en-US"`. Dates and numbers cost no bundle bytes: the CEF runtime already carries ICU data.

### 6.5 Native strings (Rust)

Same JSON files, namespace `native` (about 70 strings), embedded at compile time (`include_str!`) into the launcher and the CEF host, with the same suffix plurals and `{{name}}`. Locale: `app:locale` from `settings.json` if readable, else the OS (`GetUserDefaultLocaleName`, `NSLocale`, `LANG`). Dialogs shown before settings exist use the OS locale. This **replaces the analysis' Fluent recommendation**: the native surface has almost no plurals, and one file format keeps one translation pipeline and one coverage check. Fluent stays the fallback if native copy ever needs grammar rules.

### 6.6 Pseudo-locale and CI

- Pseudo-locale `en-XA` (accented, 30% longer, bracketed) in dev and a CI render test of the main views that fails on visible text not coming from the dictionary.
- Gates, added as warnings first and ratcheted per directory as areas are migrated (the repo's grep-gate pattern in the `docs` job): `eslint-plugin-i18next` `no-literal-string` on JSX; key parity across locales (a missing key in a wave locale warns, in a *covered flow* fails); placeholder and plural-form consistency between source and translation; a glossary term check.

### 6.7 Errors, by audience

The 456-site conversion is the wrong target. Split by who reads the text:
1. **Read by an agent or a log** (MCP tool results, hook output, system prompts, tool descriptions): English, always. Models handle English best, and a Chinese system prompt reportedly costs thousands of extra tokens (a Kimi issue comment, #4458). The agent explains it to the person in the person's language.
2. **Read by the person, with a code** (`AmxCode` through `frontend/app/errors/`): the catalog is already shaped like a dictionary; translate it, and add codes for the most common failures (target 100 in milestone 1, chosen in Phase 0 from a sample of real error banners and the srv log, since no central error report is collected today).
3. **Read by the person, legacy raw string** (`AMX-LEGACY`): show a translated frame ("Something went wrong") and the original English as **Technical details**, instead of echoing it as the whole message.

Loap-3's in-flight one-audience-tag spec may supply the person-versus-agent split; check it in Phase 0.

### 6.8 Layout, input and IME (release gates)

Text expansion to +30%; no fixed-width labels; truncation always has a tooltip. The composer must not submit on Enter while composing (`isComposing`), and the terminal and editor panes are tested with Japanese, Chinese and Korean IME on all three platforms. RTL: `dir="auto"` on user-content fields from wave 1; the physical-to-logical conversion of the 338 declarations happens file by file as areas are touched, with a stylelint rule banning new physical properties; the full RTL pass is wave 3.

## 7. Translation workflow and what each language costs

### 7.1 Workflow

1. A PR changes `locales/en.json`. CI computes the missing and changed keys per locale.
2. A scheduled **AI draft** (our own agents on our own accounts, with the glossary and key context in the prompt) opens a PR of drafts per locale, each key marked `needs-review`.
3. A **native-speaking developer** reviews the draft: in the translation platform, or in the PR. Reviewed keys are stamped with the hash of the English they were reviewed against; an English edit marks them stale.
4. Release: a locale ships whatever is reviewed; unreviewed or stale keys fall back to English per key. Nothing blocks on a reviewer. A **string freeze** three days before release lets drafts and reviews land.
5. Glossary (one file, per language, owned by its reviewer). **Never translated:** AgentMux, provider and model names, CLI commands, file paths, settings keys, env vars, MCP tool names, code. **Feature names stay English in milestone 1** (Swarm, Vault, Kanban, Warden, Armory, jekt) as proper nouns; common words (agent, pane, tab, window, settings) are translated with a glossary entry per language.

### 7.2 Cost

All word counts are estimates; Phase 0 measures them.

| Scope, per language | Source words | Hybrid AI + human review, as a share of scope A |
|---|---|---|
| **A**: app UI (2,500 to 4,500 strings, about 6 words) 15k to 27k; native 0.5k; 100 coded errors 2.5k; landing 3.5k; store listings and release highlights 2k | 24k to 36k | **1** (the unit) |
| **B**: docs getting-started set (about 8 pages) | about 15k | **about 0.4 to 0.6** |
| **C**: the rest of the docs (about 70k words of prose) | about 70k | about 2 to 3 (later, or machine-translated with a visible notice) |
| AI drafting itself, one full pass | all of the above | a small fraction of one percent of A |
| Ongoing, per release | about 300 to 800 new or changed words | about 1 to 3 percent of A per language |

**Wave 1 is three languages of A + B; wave 2 is four, less where the translation memory repeats.** Reviewers are the real cost; the model cost is small. The reviewer budget in currency is kept in the private record, not in this public repository.

**Platform.** The app repository is public, so **Crowdin for Open Source** (free, needs approval), Weblate's Libre plan or Tolgee's free open-source plan are options at no cost; a paid team plan is the fallback if the free plan is refused. The landing and cloud sources live in separate private repositories: their strings (a few thousand words) go through the same JSON files and pull requests, with no platform. **The operator and AgentY approve any paid platform or reviewer** (decisions 3 and 4).

## 8. Phases and the first milestone

Estimates are in engineer-weeks from the analysis, re-based on today's counts. The agent fleet runs the extraction in parallel by area, so wall-clock is shorter.

### 8.1 Phases

| Phase | Scope | Effort |
|---|---|---|
| **0 Spike** | Measure the real string count; confirm per-locale chunks load from CEF on three platforms; pick the getting-started docs set; sample 50 real user-visible errors for the audience split; close the data gap (4.3); lock library and keys | 1 to 1.5 weeks |
| **1 Framework** | Dictionary loader, `core` namespace, `app:locale` setting and picker, `format.ts`, pseudo-locale, lint gates in warn mode, the glossary file, the translation PR bot | 1.5 to 2 weeks |
| **2 Extraction** | By area, one PR each, ordered by what a new person sees first: first run and sign-in, agent pane and composer, settings, panes and tabs, Swarm, the rest. Ratchet the lint gate per directory | 6 to 12 weeks of work, 3 to 5 weeks wall-clock with 3 to 4 agents |
| **3 Errors** | Translate the catalog; add 100 codes; the translated frame for legacy errors | 2 to 4 weeks |
| **4 Native** | Menu, tray, dialogs, splash; MSIX `Resources`, macOS `CFBundleLocalizations`, Linux `.desktop` and AppStream | 1 to 2 weeks |
| **5 Agent replies** | `agent:reply_language` and `agent:commit_language` settings, the marked block, the per-provider levers (3.2), the "honours it / hint only" label | 1 to 1.5 weeks |
| **6 Landing** | Locale routes, prerender, `hreflang`, sitemap, switcher | 2 to 3 weeks (in parallel from Phase 1) |
| **7 Docs** | Starlight `locales` and fallback; the getting-started translations | 1 week plus review |
| **8 Stores and release notes** | Listings, winget locale manifests, highlight translation step in the release script | 1 week |
| **9 Wave 2 languages** | One week each once the pipeline exists | 4 weeks |
| **10 RTL** | Remaining logical-property conversion, mirrored icons, `ar` pilot, native-speaker QA | 3 to 5 weeks |

**Milestone 1 (Phases 0 to 8 for three languages): about 14 to 22 engineer-weeks, 6 to 9 weeks wall-clock.**

### 8.2 The completeness tradeoff, stated plainly

A language is **Beta** until, for its *covered flows* (install, first run, sign-in, open an agent, panes and tabs, settings, the top 100 errors), at least 98% of keys are reviewed. A Beta label shows in the picker. Below the gate the app still works: missing keys fall back to English one by one, never blank. What stays in English in milestone 1, by design: terminal text from the hosted CLIs (we cannot change it), the long tail of error text (translated frame, English details), agent-facing text, and all docs outside the getting-started set. The analysis' risk 5 (a language supported in name while errors stay English) is handled by naming it Beta and by the translated error frame.

## 9. Fix now, regardless of this spec: one small bug PR

Bug fixes are allowed under the current feature freeze. One PR:
1. `app-update.ts:79` and `InputHistorySearch.tsx:29` pass `"en-US"` to `toLocaleDateString`, forcing US dates for everyone: pass `undefined` (the person's locale).
2. Replace the hand-built relative times in `usenotification.tsx`, `HistoryUsagePopover.tsx` and `lan-peer.ts` with one `formatRelativeTime` on `Intl.RelativeTimeFormat` (correct plurals in every language); the devtools panel may stay.
3. Tests for both. No new dependency. This is the seed of the formatters (6.4). **Done:** agentmux#6363.

## 10. Risks

1. **Extraction volume.** 2,500 to 4,500 strings across 512 components; the count is the least certain number here. Phase 0 measures it before any schedule is promised.
2. **Reviewer supply and quality.** A native developer per language is the long pole. Mitigation: three named reviewers before Phase 2 starts, a glossary from day one, and the Beta rule.
3. **Drift after the first release.** Every UI-copy PR now has a translation tail. The AI draft bot and the per-key fallback keep that off the critical path; the stale-hash rule keeps translations honest.
4. **Hosted CLIs stay English.** A person using Claude Code in a Japanese AgentMux still sees English permission prompts. We say so in the provider settings page.
5. **IME on three platforms.** The most likely source of embarrassing bugs (every CLI vendor has them). It is a release gate, not a follow-up.
6. **Provider facts change monthly.** The lever table (3.2) is per-provider data that can be updated without a release, not code.
7. **Landing and SEO.** Wrong `hreflang` or an IP redirect can hurt search ranking in every language; follow the rules in section 5 row 3.

## 11. Decisions for the operator

1. **Wave 1 languages.** Recommended: Japanese, Simplified Chinese, Brazilian Portuguese. Alternatives: swap in Korean, or Spanish, if you know your audience better than the public data does.
2. **Docs in milestone 1.** Recommended: getting-started set only (about half the cost of the app scope per language), the rest English with the notice. Alternative: all docs now (two to three times the app scope more, per language).
3. **Paid reviewers.** Recommended: approve a budget for one native-speaking developer reviewer per wave 1 language (figure in the private record). Alternatives: volunteers only (free, slower, quality risk), or AI-only with a "machine translated" label.
4. **Translation platform.** Recommended: apply to Crowdin for Open Source (free). If refused, Weblate or Tolgee free open-source plans, then a paid plan.
5. **Beta policy.** Recommended: ship under-covered languages as Beta with per-key English fallback, instead of waiting for completeness.
6. **RTL timing.** Recommended: after wave 2, with groundwork (logical properties, `dir="auto"`) done as areas are touched.

Not decisions, but flagged: the one bug PR (9) can start now; the build tasks wait for approval and for AgentY to lift the feature freeze.

## 12. Sources

### 12.1 Cited

- Previous analysis: `docs/analysis/ANALYSIS_I18N_INTERNATIONALIZATION_SCOPING_AND_COST_2026_09_07.md`.
- GitHub Octoverse 2026: https://github.blog/news-insights/octoverse/octoverse-commits-pushes-reach-new-highs-in-2026-as-developers-build-with-agents/
- Stack Overflow Developer Survey 2026: https://survey.stackoverflow.co/2026/work/data and https://stackoverflow.co/company/press/archive/stack-overflow-annual-developer-survey-ai-trust-conditional
- Anthropic Economic Index (March 2026): https://www.anthropic.com/research/economic-index-march-2026-report
- Claude Code settings (`language`): https://code.claude.com/docs/en/settings-reference ; Claude apps languages: https://support.claude.com/en/articles/10769299-how-to-use-claude-in-your-preferred-language ; issues in `anthropics/claude-code`: #29147, #50680, #64472, #72257, #59299, #99800, #36500, #48257, #89489.
- Codex: https://developers.openai.com/codex/cli/reference and https://developers.openai.com/codex/config-advanced ; issues in `openai/codex`: #26136, #24741, #51926, #5113, #8572. ChatGPT language: https://help.openai.com/en/articles/8357869-how-to-change-your-language-setting-in-chatgpt
- Gemini CLI configuration https://geminicli.com/docs/reference/configuration.md ; issues in `google-gemini/gemini-cli`: #17814, #1871, #2487; Gemini app: https://support.google.com/gemini/answer/15984485
- Qwen Code language docs: https://qwenlm.github.io/qwen-code-docs/en/users/features/language/ and settings https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/ ; `QwenLM/qwen-code` #2830, #6943, #6953.
- Kimi Code: https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/config-files ; `MoonshotAI/kimi-code` #3571, #4607, #4458, #2575.
- Copilot CLI config: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference ; `github/copilot-cli` #3681, #4407. Copilot Chat settings: https://code.visualstudio.com/docs/agents/reference/ai-settings
- VS Code locales: https://code.visualstudio.com/docs/configure/locales ; command line https://code.visualstudio.com/docs/configure/command-line ; l10n: https://github.com/microsoft/vscode-l10n ; `microsoft/vscode` #151200, #174848.
- Cursor: https://forum.cursor.com/t/how-to-set-the-language-of-the-software-interface/164137 ; https://forum.cursor.com/t/why-cant-i-set-the-output-language-of-cursor-to-simplified-chinese-anymore/161872 ; rules https://cursor.com/docs/context/rules
- Aider options: https://aider.chat/docs/config/options.html ; Cline #12518, #7055; OpenCode #37216; Crush #4021; Goose #2376.
- `@solid-primitives/i18n`: https://github.com/solidjs-community/solid-primitives/blob/main/packages/i18n/README.md
- Platforms: Crowdin, Lokalise, Weblate, Tolgee pricing pages (2026 comparisons: https://webtranslateit.com/learn/compare/best-translation-management-software , https://danian.co/articles/post/open-source-crowdin-and-lokalise-alternatives-2026).

### 12.2 Not verified, to settle in Phase 0

- Claude app language count (11 in the Help Center, 33 in a 2026-10-29 secondary report; the primary post would not load).
- Kimi's `AGENTS.md` paths (the docs page was 404 when fetched; paths come from search summaries).
- Cursor's `argv.json` location (inferred from VS Code).
- Several GitHub issue details come from fetch-and-summarise steps, not full threads; the research notes in the task scratch mark them.
- Korea's top-ten place by pushes comes from a secondary source; the Octoverse page lists only five countries by pushes.
- String and word counts (section 2 and 7.2) are estimates from regex counts.

## 13. Build log (the record of this work; newest last)

Decisions, 2026-11-04, the operator's words relayed by AgentY: "great, proceed with best recommendations on the 2 specs (multi-language and animations)". All six decisions in section 11 take the recommended answer. AgentX's product read added three rules, which this spec now carries:
1. **Inside the agent pane only AgentMux's own labels translate** (row kinds, verbosity, buttons, system notices). Transcript text, tool output and the agent's words never do, and a Beta language's picker label says so.
2. **Feature names stay English as product names** (Swarm, Vault, Kanban, My agents, Jekt, Warden, Armory) and are in the glossary as do-not-translate; the generic words around them translate. Agent names and slugs never translate.
3. **The reply-language setting shows the marked instruction it adds** to the agent's prompt, not hides it.
Wave 2 keeps Spanish first and lets the Phase 0 landing-log data reorder it.

| Date | Who | What | Proof |
|---|---|---|---|
| 2026-11-04 | Clamk@narko | Phase 1 first step: the i18n runtime (`frontend/app/i18n/`: locale negotiation, `{{name}}` and plural lookup with English fallback per key, the reactive store, pseudo-locale `en-XA` in dev), the English dictionary seed, the glossary, the dictionary checks (keys, placeholders, plural forms, coverage), and the `app:locale` setting in the schema | this PR |
| 2026-11-04 | Clamk@narko | Phase 1 second step: the Language picker in Settings › Appearance (`app:locale`; shown once a language ships, always in dev mode for the pseudo-locale; a shipped language is labelled Beta), `<html lang>` follows the app's language, dates and relative times take the chosen language's format (`formatLocale`), the setting is bound at startup so `i18n.ts` imports no store. The notifications' relative times, the history storage note and the input-history search use it; the update-check line and LAN peer row follow after #23821 | this PR |
| 2026-11-05 | Clamk@narko | Phase 2 first extraction: the AgentMux Cloud sign-in (the choices, the scope notes, the Settings row, the Connect panel's lines, the GitHub gate's note) is read from the dictionary; `Rich` renders `<strong>`, `<code>` and `<em>` in a translated string without innerHTML, and what fills a `{{name}}` can never become markup; a check that every key the code uses is in `en.json` | this PR |
| 2026-11-05 | Clamk@narko | The literal-text ratchet (spec 6.6, as a test because it parses each file and needs no install step): a file that imports the i18n module may hold no literal visible text (JSX text, or a title, aria-label, placeholder, alt, label, description or tooltip string) beyond its number in `frontend/app/i18n/literals.baseline.json`, which can only go down; four files are listed at 45 pieces in all | this PR |
| 2026-11-10 | Clamk@narko | Phase 2 second extraction: the first-hour New agent surface is read from the dictionary (the picker's empty state, notices and template menu, the filter bar and the Free/Local/Installed chips, the create form, the harness cards' ribbons and tags, the in-app login panel, the Free sign-up guide's heading and button, and the access tags' own "checked" and "tested" lines; 80 keys). Vendor claims stay English with their source (`provider-access.ts` lead, limits, catches and steps), and the container-image notes (`containerImageNote`, `developerImageNote`) are the next piece. Literal text in the agent view fell from 434 to 370 pieces | this PR |
| 2026-11-10 | Clamk@narko | Phase 2 third extraction: the container-image notes under the runtime picker and the image choice (`containerImageNote`, `developerImageNote`), the Build cache section of the Container limits panel (with a plural for the volume count), and the Environment panel (its labels, the ignored-line and reserved-name notices, and the reasons the parser gives for a skipped line). 24 keys. Next: Settings and the pane chrome | this PR |
| 2026-11-11 | Clamk@narko | Phase 2 fourth extraction, the first of Settings: the mechanism and the first section. A settings registry entry now names its text by dictionary key (`settingEntry({ labelKey, descriptionKey, groupKey, ... })` in `settings-model.ts`), and the label, description and group are read from the dictionary each time they are read, so a row and a search result follow the language without rebuilding the index. The keywords stay as written (English synonyms and the setting key), so an English search term still finds a row in any language. Also the section rail labels, the Appearance section (12 rows and its two group headings), the Language row, the search placeholder and empty text, and the footer and error-banner buttons. About 133 entries in 25 section files are left; each is one `node` rewrite of its registry plus its section's own row text. Next: Animations, Window, Browser, Terminal | this PR |
