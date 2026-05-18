# SPEC: Optimistic Browser-Pane Header on Navigation

**Status:** Draft
**Date:** 2026-05-18
**Author:** AgentA
**Related:** `SPEC_BROWSER_PANE_FAVICON_TITLE_2026-05-15.md` (the live-update plumbing), `frontend/app/store/browser-pane-state/reducer.ts`

---

## 0. TL;DR

When the user clicks a link or navigates to a new URL, the browser pane header shows the **previous page's title** for 500ms–3s while CEF loads the new page and eventually emits `title-change`. The favicon is already optimistic (derived from the URL at `Navigate` time, t=0); the title isn't.

Fix: in the `Navigate` and `UrlConfirmed` reducer cases, also set `state.title = deriveTitlePlaceholder(url)` so the header shows the URL's hostname (e.g. `"x.com"`) immediately on click. CEF's real title replaces it later. Mirrors how Chrome shows a tab title during navigation.

---

## 1. Problem

Trace from the 2026-05-18 self-smoke:

| t | event | header state |
|---|---|---|
| 0 | user click → `Navigate` → reducer: `url=x.com, faviconUrl=derive(x.com), loading=true` | favicon ✓ x.com derived, **title still says "Google"** |
| ~100ms | CEF nav-state → `UrlConfirmed` | — |
| ~500ms–3s | CEF title-change → `TitleChanged` | title finally "X / Twitter" |

So the favicon and title are out of sync for the first ~1 second of every navigation. The favicon updates instantly; the title lags. Visible UX bug.

---

## 2. Fix

Add a `deriveTitlePlaceholder(url): string` helper in `types.ts` alongside `deriveFaviconUrl`. Returns the URL's hostname stripped of `www.` prefix; empty for unparseable URLs.

```ts
export function deriveTitlePlaceholder(url: string): string {
    if (url === "") return "";
    try {
        const u = new URL(url);
        if (u.origin === "null" || u.origin === "") return "";
        return u.hostname.replace(/^www\./, "");
    } catch {
        return "";
    }
}
```

Wire it into two reducer cases:

1. **`Navigate`** — explicit navigation from the address bar or `model.navigate(url)`. Set `title = deriveTitlePlaceholder(command.url)` alongside the existing URL + faviconUrl + faviconOverridden reset.

2. **`UrlConfirmed`** — CEF's nav-state event (covers in-page link clicks). Only update title when `originChanged`, mirroring the favicon-override logic from PR #905. Same-origin redirects/hash-changes keep the real title.

`TitleChanged` already overrides whatever the placeholder set. No change needed there.

### 2.1 Empty / null-origin URLs

`about:blank`, `file://`, etc. → `deriveTitlePlaceholder` returns `""`. Reducer falls back through `TITLE_FALLBACK` ("Browser") via the existing `TitleChanged` empty-check path. Net: those URLs show "Browser" optimistically, same as today.

---

## 3. Test plan

- [ ] Type `x.com` → press Enter. Header instantly reads "x.com" (was "Google" or whatever previous page). Real title "X / Twitter" replaces it ~1s later.
- [ ] Click an in-page link to a same-origin path (e.g. `x.com/home`). Title stays at the real "X / Twitter" — no flash to "x.com" placeholder.
- [ ] Click an in-page link to a different origin (e.g. opens `t.co/...`). Title flashes to "t.co" then upgrades to real title.
- [ ] `about:blank` shows "Browser".

---

## 4. Out of scope

- **Spinner / loading indicator on the title.** Could prefix with `"⏳ "` during `state.loading=true` — separate UX call, not in this spec.
- **Address-bar visual feedback for invalid URLs.** Outside header.

---

## 5. Acceptance criteria

1. `Navigate` dispatch produces a header that's immediately self-consistent (favicon + title both derived from the target URL).
2. Cross-origin nav from CEF nav-state (in-page clicks) also resets the title placeholder.
3. Same-origin redirects/hash-changes preserve the real title (no flash).
4. `TitleChanged` still wins once CEF emits.
