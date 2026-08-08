# SPEC: Early Alpha Warning — README & Microsoft Store Partner Center

**Date:** 2026-06-05
**Status:** Proposed (implemented — see note below)
**Owner:** TBD

> **2026-08-07 audit note:** Implemented — `README.md` directly cites and
> implements this spec by name. Status field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Scope:** Top-of-README banner + Microsoft Store Partner Center listing copy

---

## Motivation

AgentMux is publicly available (GitHub README, Microsoft Store listing) but the
product is still in **early alpha**: significant feature surfaces are partially
implemented, regress between releases, or are known-broken on specific
platforms. Users arriving from the Microsoft Store in particular have no signal
that they are installing pre-production software — the Store listing reads as a
finished product, which produces unfair 1-star reviews and wastes the user's
time.

This spec defines a single, consistent **early-alpha warning** that lands in
two places:

1. **Top of `README.md`** in the `agentmuxai/agentmux` repo (the first thing a
   GitHub visitor or installer-link follower sees).
2. **Microsoft Store Partner Center listing** — both the Description and the
   "What's new in this version" copy, so the warning is unmissable on the
   product page *and* on every update notification.

The warning must be prominent, honest, and self-consistent across both surfaces
so that the Store reviewer, the GitHub visitor, and the installed-app user are
all looking at the same disclaimer.

## Goals

- A user reading either surface immediately understands: this is alpha, things
  break, and bugs should be reported as GitHub issues.
- The two surfaces share one canonical wording — when we change one, we change
  the other.
- The warning survives README restructuring (i.e., it's the first content
  block, above logo niceties and badge rows if necessary).
- The warning is reusable: the `AppxManifest.xml.template` Description field
  references the same text.

## Non-Goals

- Changing the in-app UI to show an alpha banner (separate spec if we want
  that).
- Adding telemetry/feedback widgets.
- Renaming the product or version-scheme changes.

## Wording constraint — Store review policy

The warning **must not** tell users to file a GitHub issue *instead of* leaving
a review (e.g. "rather than leaving a review", "don't leave a review"). The
Microsoft Store has a ratings/reviews-manipulation policy and copy that
discourages reviews can trip Store certification, especially in shipped
artifacts (`AppxManifest.xml.template` Description, Partner Center listing
fields). Phrase the GitHub-issue ask positively: *"please report issues at
…"* — link to the issue tracker but do not contrast it with the Store review
flow.

---

## Canonical Warning Text

The single source of truth lives in this spec. All other surfaces copy from
here verbatim (or paraphrase only the Markdown/plain-text formatting around
it).

### Short form (one line, used in Store "Short description" and any tight slot)

> **Early alpha — expect bugs, broken features, and breaking changes between
> releases. Please report issues at https://github.com/agentmuxai/agentmux/issues.**

### Long form (used at top of README and in Store "Description")

> ## ⚠️ EARLY ALPHA — Use At Your Own Risk
>
> **AgentMux is in early alpha.** Many features are incomplete, partially
> broken, or change between releases without notice. Expect:
>
> - **Broken features** — pieces of the UI may not function, or may regress
>   from one release to the next.
> - **Data loss** — settings, pane layouts, and agent state may not migrate
>   cleanly across versions. Don't store anything you can't reproduce.
> - **Breaking changes** — config files, identity bundles, memory bundles,
>   and the App API may change shape with no migration path during alpha.
> - **Platform gaps** — Windows is the primary target; macOS and Linux
>   builds lag behind and have additional known issues.
>
> If you hit a problem, **please report it as a GitHub issue** at
> https://github.com/agentmuxai/agentmux/issues — it's how alpha gets to beta.

---

## Surface 1 — `README.md`

### Placement

Insert the **Long form** warning as the very first content block, **above** the
centered logo `<p align="center">…</p>` block, so it is the first thing visible
both on github.com and in any tool that renders raw Markdown without centering.

Rationale: the logo+title is decorative; the warning is load-bearing
information. Decorative content should not push the warning below the fold.

### Exact edit

Prepend to `README.md` (at the repo root):

```markdown
> ## ⚠️ EARLY ALPHA — Use At Your Own Risk
>
> **AgentMux is in early alpha.** Many features are incomplete, partially
> broken, or change between releases without notice. Expect:
>
> - **Broken features** — pieces of the UI may not function, or may regress
>   from one release to the next.
> - **Data loss** — settings, pane layouts, and agent state may not migrate
>   cleanly across versions. Don't store anything you can't reproduce.
> - **Breaking changes** — config files, identity bundles, memory bundles,
>   and the App API may change shape with no migration path during alpha.
> - **Platform gaps** — Windows is the primary target; macOS and Linux
>   builds lag behind and have additional known issues.
>
> If you hit a problem, **please report it as a GitHub issue** at
> https://github.com/agentmuxai/agentmux/issues — it's how alpha gets to beta.

---

```

(The trailing `---` separates the warning from the existing logo/title block.)

### Acceptance

- `README.md` rendered on github.com shows the warning callout *before* the
  logo and title.
- The warning uses GitHub's blockquote rendering (with the `>` prefix) so it
  visually stands apart from the rest of the README.
- No existing content is removed; everything currently in the README slides
  down unchanged.

---

## Surface 2 — Microsoft Store Partner Center

### Fields to update

Partner Center → AgentMux → **Store listings → English (United States)** (and
any other locales we publish):

| Field | New value |
|---|---|
| **Short description** (≤200 chars) | Short form (see above) — prepended to existing description. |
| **Description** (≤10 000 chars) | Long form (see above) at the very top, followed by the existing marketing copy. |
| **What's new in this version** | Short form on its own line at the top, then the actual release notes for the version. Re-applied on every Store submission until we exit alpha. |

### Submission procedure

1. Sign in to Partner Center → Apps and games → AgentMux.
2. Open the current draft submission (or create a new one if none is open).
3. Update Store listings → English (US) per the table above.
4. For every additional locale, replicate the same English text (we are not
   translating during alpha).
5. In the **Notes for certification** field, add: *"This submission adds an
   early-alpha disclaimer to the Store listing. No package binary changes
   versus the previous submission unless otherwise noted."*
6. Submit for certification.

### Acceptance

- Store listing preview shows the long-form warning as the first paragraph of
  the Description.
- The Short description visible in search results contains the short-form
  warning.
- The "What's new" section on the product page shows the short-form warning
  above the release notes.

---

## Surface 3 — `AppxManifest.xml.template` (consistency check)

`packaging/msix/AppxManifest.xml.template` contains the `<Description>` element
that Microsoft Store falls back to if the Partner Center listing field is
empty. Update it so the two surfaces cannot drift apart:

- Set `<Description>` to the **short form** warning followed by `" — "` and
  the existing one-line product pitch.

This is a belt-and-braces measure; Partner Center copy normally wins, but a
mis-configured submission could fall back to the manifest text, and we want
that fallback to still carry the warning.

### Acceptance

- Re-running `packaging/msix/generate-assets.ps1` (or whatever step renders
  the manifest from the template) produces a manifest whose `<Description>`
  starts with the alpha warning.

---

## Out-of-band: keeping the surfaces in sync

When the warning text changes (e.g., when we drop "early alpha" for "beta"):

1. Edit this spec's **Canonical Warning Text** section.
2. Update `README.md` to match.
3. Update `AppxManifest.xml.template` to match.
4. Open a Partner Center submission with the new Store-listing copy.

A future automation could lint the README and manifest against this spec's
canonical block, but during alpha the manual review at submission time is
adequate.

---

## Open questions

- Do we want a matching warning on the agentmux.ai landing page? (Out of scope
  here; flag for the website owner.)
- Do we want an in-app first-run banner echoing the warning? Defer to a
  follow-up spec if user-research signals it's needed.
- Localized Store listings — do we want machine-translated warnings in the
  other locales, or keep English everywhere during alpha? Current spec says
  English-only for simplicity.
