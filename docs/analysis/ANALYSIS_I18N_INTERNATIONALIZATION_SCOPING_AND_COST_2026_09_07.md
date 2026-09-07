# Analysis: Internationalization (i18n) — scoping, architecture options, and resource cost

**Date:** 2026-09-07
**Status:** proposed — analysis complete, not yet implemented
**Author:** AgentX

---

## 1. Summary

AgentMux has **zero i18n infrastructure today** — no `i18next`, `solid-i18n`, `rust-i18n`/`fluent`, or `gettext` anywhere in the tree (confirmed by grep across the 6 Rust crates and the frontend). Every user-facing string, in both the SolidJS frontend and the native Rust host, is hardcoded English. This is a **retrofit**, not a greenfield build, and the research below is consistent on what that means: retrofitting i18n into an existing codebase runs **2–5x** the cost of building it in from day one, because it requires refactoring UI components, error-handling code, and layout assumptions that were never designed to be swapped out — not just running a string-extraction script.

**Total estimated engineering effort: roughly 15–37 engineer-weeks (≈3.5–8.5 months for one senior engineer, ≈2–5 months with a 2-person team splitting frontend/backend work), plus $2,400–$30,000 in one-time translation cost and $1,800–$6,000/year in translation-platform subscription for an illustrative 4-language launch (Spanish, German, Japanese, Arabic).** Full breakdown in §4–§6. The single biggest schedule risk is not the frontend — it's that **456 backend error sites** construct raw English strings that cross the wire verbatim today (§3.3), and a frontend-only effort would ship an app whose most consequential text (error messages) is still unlocalized.

This document is a scoping and cost analysis only. Nothing has been implemented.

---

## 2. Scope of this analysis

Per direction from the repo owner: this analysis costs a **full phased rollout** (framework + extraction + RTL + translation vendor), broken into phases so a smaller first milestone can be chosen without re-deriving the numbers. The **illustrative locale set is Spanish, German, Japanese, and Arabic** — one high-volume Western European language, one CJK language (different script, no plural complexity but real layout/font implications), and one RTL language — chosen to make the cost model concrete, not as a commitment to ship those four specifically. All per-locale costs in §6 are pro-rata; substitute your own target list.

Methodology: a read-only codebase survey (Explore agent, 53 tool calls, direct greps and file reads — findings below cite file paths and counts, with extrapolated figures explicitly marked as such) plus web research on 2026 i18n tooling and vendor pricing (sources at the end).

---

## 3. Current-state findings

### 3.1 Frontend (SolidJS/TypeScript) — 602 non-test files, no central strings file

- **No existing strings/constants file.** The only "constants" file found holds numeric UI-timing values, not copy.
- **Directly counted:** 264 `title=`/`aria-label=`/`placeholder=` attributes, 100 `throw new Error("...")` sites with English text across 73 files, 59 `message:` literal payloads (toasts/notifications) across 22 files.
- **Heuristic (regex-based, not exact):** ~310 JSX text nodes, ~1,351 quoted strings tree-wide (this last figure overcounts — it includes non-copy strings like class names).
- **Extrapolated total: roughly 1,500–3,000 distinct user-facing strings.** This is a range, not a count; both the miss rate (multi-line/template-literal copy) and overcount rate (non-UI strings) work against precision here.
- **Density is uneven and file-dependent**, not domain-dependent: sampling 9 representative view files across agent/browser/terminal/editor/settings/armory/swarm found 1–15 strings per file with no pattern by feature area.
- **Easy-extraction files already exist**, and this matters for phase sequencing: `frontend/app/errors/catalog.ts` and its siblings (`accounts-catalog.ts`, `oauth-catalog.ts`, `cli-catalog.ts`, three provider catalogs, `mcp-preload-catalog.ts`) are already `Record<string, {title, message, retry}>` keyed by stable codes — a near-drop-in shape for an i18n key table.
- **Hard-extraction files** are the large, deeply-nested views — `swarm-view.tsx` (1,041 lines) and `editor-view.tsx` (961 lines) have copy inline in JSX markup with no existing key structure.

### 3.2 Native/OS-level strings (Rust host) — small in count, large in complexity

Roughly **50–70 strings across ~6 files** — small, but concentrated in the highest-stakes text in the app:

| Surface | File(s) | Count |
|---|---|---|
| macOS menu bar | `agentmux-cef/src/macos_menu.rs` | ~30 labels (File/Edit/View/Window, Undo/Redo, Zoom, etc.) |
| Tray menu | `agentmux-launcher/src/tray/mod.rs` | 3 |
| Fatal error dialogs | `agentmux-launcher/src/supervisor/{unix,windows}.rs`, `main.rs`'s `show_fatal_dialog` | 10 call sites, each a distinct multi-sentence title+body |
| OOM dialog | `agentmux-launcher/src/mem_supervisor.rs` | 2 (title+body) |
| CEF-init failure dialog | `agentmux-cef/src/lib.rs` (Windows `MessageBoxW`) + `linux_sandbox.rs` (`zenity`/`kdialog`) | multi-paragraph, 2 platform-specific implementations |
| Splash screen | `agentmux-launcher/src/splash.rs` | concrete strings (e.g. `"Restoring session..."`) via a software glyph renderer on Windows; macOS uses native `NSTextField` |

The context menu is **not** a cost center — it deliberately delegates to the frontend DOM menu and carries zero native strings. But the fatal-dialog and CEF-init strings are multi-sentence prose across **three separate native dialog subsystems** (Windows `MessageBoxW`, a macOS equivalent, Linux `zenity`/`kdialog`), so each string needs three renderer-specific wiring paths, not one.

### 3.3 Backend error strings — the real cost center, and a completeness blocker

A real structured-error catalog exists: `agentmux-common/src/errors.rs`'s `AgentMuxError` with 16 typed `AmxCode` variants, serialized as `{code, message, details}` and rendered by `frontend/app/errors/{catalog.ts, translate.ts}` — this path is cheap to localize, since only the frontend catalog needs translation.

**Adoption of that path is thin.** Direct counts in `agentmux-srv/src`:

- **3** call sites use `AgentMuxError::` directly.
- **67** handlers are still typed `Result<_, String>`.
- **456** sites construct a raw string in error position (`format!`/`.to_string()`).

All 456 fall through `AgentMuxError::Legacy(String)` → wire code `AMX-LEGACY`, and the frontend catalog entry for `AMX-LEGACY` **echoes the raw backend English text verbatim** — it does not translate it, because there's nothing structured to translate. A concrete example: `agentmux-srv/src/identity/resolver/errors.rs`'s `SpawnGateError::MissingCredentials` builds multi-sentence prose that its own doc comment says is surfaced "`Display` verbatim in the agent pane."

**This is the finding that should drive sequencing.** A frontend-only i18n effort translates the UI chrome — buttons, menus, the ~1,500–3,000 strings in §3.1 — while the error messages a real user actually reads when something breaks stay in raw English indefinitely, because they're not strings-that-need-translating, they're **prose baked into 456 call sites of Rust error-construction code**. Closing this gap means touching error-handling logic across the entire `agentmux-srv` crate, not swapping strings for keys.

### 3.4 Formatting

- 22 `toLocaleDateString`/`toLocaleTimeString`/`toLocaleString` call sites across 18 files. **Zero** uses of `Intl.NumberFormat`/`Intl.DateTimeFormat`.
- **Two sites hardcode `"en-US"`** explicitly (`statusbar/InstancePanel.tsx:68`, `statusbar/MaintenanceSection.tsx:33`) — these force US formatting regardless of OS locale today and need a direct fix regardless of the broader i18n timeline. Most other sites already pass `undefined` correctly (OS-locale-respecting).
- One `dayjs` usage with a hardcoded format string (`"h:mm:ss A"`, sysinfo plot).
- **`frontend/app/notification/usenotification.tsx` hand-builds relative-time copy** ("Just now", "X mins ago", "X hrs ago", "X days ago") with hardcoded English singular/plural and no ICU plural rules. This is the single highest-cost formatting item found: English's 2 plural forms don't generalize — Polish has 4, Arabic has 6, Russian's rule depends on the last two digits of the number — so this code needs a real plural-rules library, not a bigger switch statement.

### 3.5 RTL readiness

Of 147 `.scss` files, **29 files (57 declarations) use physical `left:`/`right:` positioning; only 1 file uses a logical property** (`margin-inline`-style). RTL support means auditing and converting most of those 29 files, plus wiring the HTML `dir` attribute and auditing directional icons — none of which is currently in place anywhere in the codebase.

### 3.6 Build/tooling constraint — this is an architecture decision, not a checkbox

`vite.config.ts` **explicitly disables `manualChunks`** — the config comment states this was done because static per-chunk splitting broke the old WebKitGTK (Linux) host, so today "all code goes in one bundle." 37 `import()` call sites exist and are lazy-loaded at the source level, but per that same constraint they inline into the single bundle rather than producing separate physical files. **This means per-locale bundle splitting into separate downloadable files is not straightforward today** — locale dictionaries would either have to ship inlined in the one bundle (bundle size grows with every locale added) or the WebKitGTK-driven `manualChunks:false` decision would need to be revisited and re-tested against the exact regression it was disabled to fix. This is flagged as a real open architecture question in §6, not assumed away.

One useful precedent exists: `platformResolve()`, a custom Vite plugin resolving `.platform.ts` imports to `.win32.ts`/`.darwin.ts`/`.linux.ts` **at build time**. It's a working model for compile-time resolution, but locale switching needs runtime resolution (a user changes their language without a rebuild), so it doesn't directly solve this — it's evidence the team has hand-rolled similar Vite plugins before, which is relevant to estimating Phase 6 below.

### 3.7 CI integration point

`.github/workflows/ci-pr.yml` already runs a `docs` job with several grep-gate scripts (`check-menu-positioning.sh`, `check-scrollbar-cursor.sh`, `check-muxbus-credential-store.sh`, etc.) — bash scripts that grep for a pattern and fail the build on a match, with an allowlist for sanctioned exceptions. A future "no raw JSX text" / "missing translation key" gate fits this exact, already-established pattern. `eslint.config.js` is currently minimal (no i18n plugin wired in), so this would be new tooling, not a config tweak.

---

## 4. Recommended architecture (from 2026 best-practice research)

| Layer | Recommendation | Why |
|---|---|---|
| Frontend (SolidJS) | **`@solid-primitives/i18n`** | Community-official, reactive-first (updates only the DOM nodes containing translated text via signals, not a virtual-DOM re-render), designed for Solid's own reactivity model rather than adapted from React. `solid-i18n` (SanichKotikov) is a lighter alternative with built-in ICU plurals/dates via native `Intl` if a smaller dependency footprint is preferred. |
| Rust (native host strings) | **Fluent (`fluent-templates`/`fluent-i18n`)**, not `rust-i18n` or `gettext-rs` | `gettext-rs` doesn't support macOS, which rules it out for a cross-platform host. Fluent moves plural/gender/grammar rules out of Rust code and into `.ftl` translation files, which matters given §3.5's finding that current relative-time logic is hand-rolled. `rust-i18n`'s plain YAML key-value model is simpler but doesn't solve plurals — given the native surface here is small (~50–70 strings), the complexity delta between Fluent and `rust-i18n` is low-risk either way; Fluent is recommended for consistency with the frontend's ICU-based approach. |
| Plural/number/date formatting | **ICU MessageFormat / CLDR plural categories**, via `@formatjs`/`intl-messageformat` on the frontend and Fluent's built-in CLDR support on the backend | Every mature i18n library implements CLDR plural rules; hand-written plural conditionals are where these bugs actually originate. Wrap the formatter call in error handling — a malformed translator-authored ICU string throws at runtime, and the standard mitigation is falling back to the raw key rather than crashing. |
| RTL | HTML `dir` attribute + CSS logical properties (`margin-inline-start` instead of `margin-left`) | Matches finding in §3.5 almost exactly — 28 of 29 offending files need this conversion. RTL is not "flip the characters"; layout, icon mirroring, and native-speaker/device testing are all required — simulator RTL behavior diverges from real devices. |
| CI enforcement | `eslint-plugin-i18next` (`no-literal-string`) + a missing-translation-key checker (e.g. `eslint-plugin-i18next-no-undefined-translation-keys` or `eslint-plugin-i18n-json`'s `identical-keys` rule) | Standard 2026 combination for catching hardcoded strings and untranslated keys in CI; slots into the existing grep-gate `docs` job pattern found in §3.7. |
| Locale-neutral APIs | Backend returns ISO timestamps + raw numeric values; client formats for the user's locale | Standard layering — but AgentMux's backend also generates user-facing prose directly (§3.3), which is the exception this pattern doesn't cover and the reason Phase 3 below is expensive. |

---

## 5. Phased engineering-effort estimate

Ranges reflect the extrapolation/measurement uncertainty already flagged in §3 — no published hour-by-hour retrofit case study exists in the research (only qualitative "2–5x greenfield cost" multipliers and a readiness-audit heuristic), so throughput assumptions are stated explicitly rather than presented as precise.

| Phase | Scope | Estimate | Basis |
|---|---|---|---|
| **0 — Architecture spike** | Pick libraries, key-naming convention, locale-negotiation strategy (OS-locale detection + user override), and resolve the §3.6 bundle-splitting question before writing extraction code | 1–2 weeks | The bundle-splitting question alone is a real design decision with a known regression history (WebKitGTK), not a formality. |
| **1 — Frontend framework + easy-extraction** | Wire `@solid-primitives/i18n`; migrate the already-key-shaped catalogs (§3.1) to translation calls | 3–5 days | Catalogs are already structured; this is close to mechanical. |
| **2 — Frontend string extraction (hard part)** | Extract and wire ~1,500–3,000 strings across 602 files, including the 264 attribute strings and the ICU-plural rework of `usenotification.tsx`'s relative-time copy; fix the 2 hardcoded `"en-US"` sites | **5–15 weeks** | At an assumed 40–80 strings/engineer-day (stated assumption, not a cited figure — large/dense files like `swarm-view.tsx` and `editor-view.tsx` will run slower than the catalog files), 1,500÷60 ≈ 25 days to 3,000÷40 ≈ 75 days. |
| **3 — Backend error-string extraction** | Convert some meaningful share of the 456 raw-string error sites (§3.3) to typed `AgentMuxError` variants with catalog entries; restructure `Display`-verbatim errors like `SpawnGateError` into structured data | **4–8 weeks** | This is the widest range in the table on purpose: the site *count* is measured, but how much shared structure exists across those 456 sites (and thus how many map to a handful of new `AmxCode` variants vs. needing bespoke handling) is not — that's exactly what Phase 0 should scope more precisely before committing to a number. |
| **4 — Native/OS strings** | Wire Fluent into the ~6 native files (§3.2): macOS menu, tray, splash, and the 3 platform-specific fatal-dialog subsystems | 1–2 weeks | Low string count, but 3 distinct native dialog APIs (Windows/macOS/Linux) each need separate wiring. |
| **5 — RTL support** | Convert the 28 remaining physical-positioning scss files to logical properties; `dir` attribute wiring; icon-mirroring audit (not measured — needs its own pass); a testing cycle with a native Arabic speaker or QA vendor | 2–4 weeks | Conversion is mechanical per research, but RTL bugs are reported as consistently invisible to LTR-fluent reviewers, so the testing tail is real, not padding. |
| **6 — Locale-switching runtime / build** | Either validate inlining all locale dictionaries into the single bundle (simpler), or re-test a selective `manualChunks` re-enable against the original WebKitGTK regression (higher-risk path) | 1 week (path a) / +2–3 weeks (path b) | Directly gated by the §3.6 finding; Phase 0 should pick the path before this phase is scheduled. |
| **7 — CI lint gates** | Add `eslint-plugin-i18next` + missing-key checker following the existing grep-gate convention | 2–3 days | Established pattern already exists in this repo (§3.7); low novelty. |

**Rollup: ~15–37 engineer-weeks total.** Sequentially, that's roughly **3.5–8.5 months for one senior engineer**. Phases 2 (frontend) and 3 (backend) are largely independent workstreams and can run in parallel across two engineers, compressing wall-clock time to roughly **2–5 months** without changing total effort.

---

## 6. Translation and ongoing cost (illustrative: Spanish, German, Japanese, Arabic)

**Translatable word estimate.** ~1,500–3,000 frontend strings at an assumed ~6 words/string ≈ 9,000–18,000 source words, plus a comparable-magnitude but unmeasured backend corpus once Phase 3 lands (native + backend strings are prose-heavy per §3.2/§3.3, likely pushing the per-word average up, not down). **Rounding to a stated planning assumption of ~12,000–25,000 total source words.**

| Model | Rate (2026 market) | Cost for 4 locales |
|---|---|---|
| Human-only | $0.15–$0.30/word | 4 × 12,000 × $0.15 = **$7,200** to 4 × 25,000 × $0.30 = **$30,000** |
| Hybrid AI + human review (2026 enterprise default) | $0.05–$0.10/word | 4 × 12,000 × $0.05 = **$2,400** to 4 × 25,000 × $0.10 = **$10,000** |
| Pure AI (not recommended alone for UI copy needing product context) | $0.001–$0.002/word | 4 × 12,000 × $0.001 ≈ **$48** to 4 × 25,000 × $0.002 = **$200** — cited for completeness, not as a realistic ship-quality option |

**Recommended: hybrid AI + human review, ~$2,400–$10,000 one-time for this 4-locale set.** This is the pattern research describes as the current enterprise default, balancing the near-zero cost of pure AI against the quality risk of shipping unreviewed machine translation in product UI.

**Translation-management platform (recurring).** Crowdin bills in "hosted words" (source words × target languages) — our estimate is 12,000–25,000 × 4 ≈ 48,000–100,000 hosted words, which roughly fits or slightly exceeds Crowdin's Team tier (~$150/mo, 50,000-word allocation) or Lokalise's Growth tier (~$499/mo). **Budget $150–$500/month ($1,800–$6,000/year)**, tier depending on actual hosted-word volume once Phase 2/3 land real counts.

**Unquantified ongoing cost — flagged, not estimated.** Every future PR that touches UI copy or adds a new error message now needs a translation update before or after merge. This is a recurring tax on shipping velocity, not a one-time cost, and needs a process decision (continuous localization via the TMS vendor's workflow vs. periodic batch translation runs) that's out of scope for this document but should be made before Phase 1 starts, since it affects whether Phase 7's CI gate blocks merges on missing keys or just warns.

---

## 7. Risks and open questions, ranked by how much they could move the estimate

1. **Backend error-string scope (Phase 3) is the widest-uncertainty number in this report.** The 456-site count is solid; how much shared structure exists across those sites is not. A short spike sampling ~30 of the 456 sites for common shapes would tighten this range significantly before committing to a schedule.
2. **The WebKitGTK/`manualChunks` constraint (§3.6) could add real time to Phase 6** if the selective-re-enable path is needed — that path directly re-tests conditions that caused a past regression on Linux.
3. **RTL icon-mirroring and layout scope wasn't directly measured** — the 29-file/57-declaration count covers CSS positioning only; a full RTL audit needs a dedicated pass over icons and asymmetric layouts that this survey didn't attempt.
4. **§3.4's two hardcoded `"en-US"` sites and the un-pluralized relative-time strings are real, narrowly-scoped bugs today**, independent of the broader i18n timeline — worth fixing regardless of whether the rest of this rollout is greenlit.
5. **Completeness gate:** shipping only Phases 1–2 (frontend chrome) without Phase 3 (backend errors) means the app "supports" a language in name while its most consequential text — what a user sees when something breaks — stays English. If a partial rollout is chosen, this tradeoff should be stated explicitly to whoever signs off on it, not discovered after launch.

---

## Sources

- [@solid-primitives/i18n — npm](https://www.npmjs.com/package/@solid-primitives/i18n)
- [I18n - Solid Primitives](https://primitives.solidjs.community/package/i18n/)
- [SolidJS i18n: Reactive Localization Guide for 2026 | IntlPull](https://intlpull.com/blog/solidjs-i18n-localization-guide-2026)
- [fluent-i18n — crates.io](https://crates.io/crates/fluent-i18n)
- [GitHub - orhun/fluent-i18n](https://github.com/orhun/fluent-i18n)
- [Rust internationalization, localization, and translation - LogRocket Blog](https://blog.logrocket.com/rust-internationalization-localization-and-translation/)
- [The complete technical guide to Internationalization (i18n) & Software localization | SimpleLocalize](https://simplelocalize.io/blog/posts/internationalization-guide-software-localization/)
- [ICU Message Format Guide: Syntax, Plurals & Real-World Examples (2026) | Crowdin Blog](https://crowdin.com/blog/icu-guide)
- [Frontend Internationalization 2026: ICU, RTL, and Locale Routing – techinterview](https://www.techinterview.org/post/3233475402/frontend-internationalization-2026-icu-rtl-locale-routing/)
- [Software Internationalization (i18n) for Engineers | LILT](https://lilt.com/blog/software-internationalization)
- [Why retrofitting i18n is expensive (and what teams discover too late) | SimpleLocalize](https://simplelocalize.io/blog/posts/why-retrofitting-i18n-is-expensive/)
- [Crowdin Software Pricing & Plans 2026 | Vendr](https://www.vendr.com/marketplace/crowdin)
- [Lokalise Software Pricing & Plans 2026 | Vendr](https://www.vendr.com/marketplace/lokalise)
- [AI vs Human Translation Cost: Cut Localization Costs by up to 97% | Lokalise](https://lokalise.com/blog/translation-cost-human-vs-ai-orchestration/)
- [eslint-plugin-i18next — GitHub](https://github.com/edvardchen/eslint-plugin-i18next)
- [eslint-plugin-i18next-no-undefined-translation-keys — npm](https://www.npmjs.com/package/eslint-plugin-i18next-no-undefined-translation-keys)
