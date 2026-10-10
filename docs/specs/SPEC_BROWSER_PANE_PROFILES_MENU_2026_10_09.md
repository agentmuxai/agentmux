# SPEC: Home and Profile buttons in the browser pane's toolbar, with Incognito and named profiles

**Status:** active — P1 (Home, the Profile menu, Incognito tabs on Windows) shipped in PR #4574; P2 (named profiles, Windows) shipped in PR #4583; P4 (agents) in PR #4585; P3 (Linux/macOS) not started.
**Author:** AgentX@narko, 2026-10-09, at the operator's request
**Builds on:** `SPEC_BROWSER_PANE_IDENTITIES_2026_09_22.md` (the model: one `browser:identity`
key per browser tab, in-memory private contexts, disk-backed named profiles, and the two CEF
constraints). This spec replaces its §5.3 UX: a tab's identity is chosen when the tab is
opened and never changes, so there is no "recreate this pane in another identity" step.
Also `SPEC_BROWSER_PANE_START_PAGE_2026_09_16.md` (the start page Home goes to).

## 1. The ask

> add a profile button to the browser address bar. Also, a piggyback fix: add the Home button
> to the right of refresh, then bookmarks, then profiles. When clicking the profile, it shows
> what profile you are in; one selection is "Open new Incognito", which opens a window pane
> with an incognito browser. The other items should be New Profile, and the list of existing
> profiles. Clicking a profile always opens a new pane tab (use the same window pane).

## 2. The toolbar

```
 ←  →  ↻  ⌂  🔖  (A)  [ https://example.com/page                         ]  →
                 │    └ Profile: who this tab is browsing as
                 └ Bookmarks (unchanged)
          └ Home: the start page
```

| Button | Does | Tooltip |
|---|---|---|
| **Home** (new, `fa-house`) | Goes to the start page (`browserStartPageAtom`, else the default page). Right-click: **Set this page as Home**. | `Home: <start page>` |
| **Bookmarks** | Unchanged. Its "Set as Start Page" item stays, renamed **Set as Home page**; "start page" becomes "Home page" in every label, so there is one name for one thing. | `Bookmarks` |
| **Profile** (new) | Opens the profile menu (§3). Its face shows the tab's identity (§4). | `Browsing as <name>` |

All three are the toolbar's existing `browser-nav-btn` style, the same size and spacing as Back,
Forward and Reload. Keyboard: **Alt+Home** goes Home, **Ctrl+Shift+N** opens an Incognito tab
(§3), both while the browser pane has focus.

## 3. The profile menu

A `FlyoutMenu` anchored under the Profile button (so it already avoids window edges and native
panes):

```
┌──────────────────────────────────────┐
│ (A)  Personal                        │  ← who this tab is, with its face
│      Your saved sign-ins              │
├──────────────────────────────────────┤
│ 🕶  Open new Incognito tab   Ctrl+⇧+N │
├──────────────────────────────────────┤
│ (A)  Personal                     ✓  │  ← every profile; ✓ on this tab's
│ (W)  Work                            │
│ (C)  Client X                        │
├──────────────────────────────────────┤
│ +   New profile…                     │
│ ⚙   Manage profiles…                 │
└──────────────────────────────────────┘
```

- **The header** says who the tab is browsing as, with one line on what that means:
  - "Your saved sign-ins" for a profile;
  - "Nothing is saved, and it's gone when the tab closes" for an Incognito tab.
- **Open new Incognito tab** opens a new tab in this pane, browsing in a fresh in-memory jar of
  its own. Every Incognito tab is separate from every other one, not a shared "incognito jar".
- **A profile in the list** opens a new tab in this pane, browsing as that profile. The tab
  you're in is left alone. The current profile has a ✓, and choosing it opens a new tab in it
  too, so "another tab as me" is one click.
- **Both open at the current page's address**, so "this site as my other account" is one click.
  From a blank or error page they open Home instead.
- **The new tab takes focus**, and the old one stays in the strip, as a browser opens a new tab.
- **New profile…** opens a small dialog: a name, and a colour picked for it that you can change.
  Enter or Create makes the profile and opens a new tab in it, at Home, since a fresh profile is
  signed in nowhere. (A dialog, not a field inside the menu: a menu's keys are its own, and a
  text field in one fights them.)
- **Manage profiles…** opens **Settings → Browser → Profiles**: rename, recolour, delete (with a
  confirm that names what will be signed out), and the order of the list.
- **The first profile is "Personal".** It's today's shared jar, so every existing tab, and
  everything you're already signed in to, is in it from day one. It can't be deleted, or renamed
  (for now).

## 4. Showing which identity a tab is in

| Identity | Profile button face | Pane-tab pill |
|---|---|---|
| A profile | a circle in its colour with its initial | the same small circle before the title |
| Incognito | `fa-user-secret`, in the accent colour | `fa-user-secret` in place of the site's icon |

The Profile button sits just before the address field, so it is the address bar's mark too: a
second glyph beside it would only repeat it.

- **Personal tabs keep their site's icon,** however many profiles there are: marking every tab
  would replace all their favicons. Only Incognito and named-profile tabs are marked, so the
  special ones stand out, and two tabs of one site side by side are told apart at a glance.
- **A named profile's tab** shows a person glyph in its colour in the tab strip; its Profile
  button wears the colour, and the menu shows its initial on its colour.
- **Hovering a tab pill** adds "Browsing as Work" (or "Incognito") to its tooltip.

## 5. Behaviour

- **A tab's identity is fixed.** It's chosen when the tab is opened, and lives in the tab's block
  meta (`browser:identity`: absent for Personal, `"incognito"`, or `"profile:<id>"`).
  Navigating, Back, reloading or redocking never changes it. To switch, open a new tab. This
  replaces the identities spec's "recreate the pane" step.
- **Popups follow their opener.** A popup pane or popup window a tab opens uses the tab's
  identity: the sign-in popup of a Work tab signs in to Work (native-popups spec §8.5).
- **Incognito tabs come back signed out.** Panes live in srv's store, so after a restart an
  Incognito tab is still there. Its in-memory jar is gone, though, so it reopens as an Incognito
  tab with a fresh jar: signed out, its address kept like any tab's. A layout reopened from
  Layouts gives its Incognito tabs fresh jars too, so they never share one with a tab still open.
  Tabs that shared a jar still share one.
- **Profile tabs are restored, still signed in,** since a profile's jar is on disk (§6). The jar
  lives in the version's CEF cache, like Personal's; the list of profiles is shared across
  versions, so after an update a profile is still there, with the same sign-in state Personal
  has after one.
- **One jar per profile:** every tab of a profile shares it. Each Incognito tab has its own.
- **Bookmarks and Home are global,** the same in every profile, for now (§9).
- **Deleting a profile** first closes its open tabs (after the confirm), then removes its data.
- **Tear-off and redock keep the tab's identity.** The host keeps an Incognito tab's jar alive
  for as long as the tab exists (identities spec §4.1).
- **Agents:** `OpenBrowser` gets an optional `profile` (`"incognito"` or a profile name).
  - **Default:** unchanged in this spec.
  - **Named profiles:** an agent may use one only after you switch on "Agents may use this
    profile" for it in Manage profiles.
  - **Incognito:** always allowed, since it gives the agent less, not more (identities spec §7).
  - **Agents never create, rename or delete profiles.** `browser_profiles.create`, `update`
    and `delete` refuse a connection registered as an agent; `list` doesn't.
  - **Where it's checked (P4):** `OpenBrowser`'s `profile` goes through
    `browser_identity::identity_for_agent`. A `browser:identity` an agent passes to `pane.open`
    (a connection registered as an agent) or to the HTTP pane open (`/api/v1/pane/open`, which
    agents and `muxsh` call) goes through `check_agent_may_use`. The window's own calls aren't
    checked, the same trust as every other key the window sets.

## 6. How it works

- **srv:**
  - **The profile registry:** `~/.agentmux/shared/browser-profiles.json`, a list of `{id,
    name, color, agents_allowed}`. "Personal" is implicit, with id `default`.
  - **Routes:**
    - `browser_profiles.list`, `create`, `update`, `delete`, the way the start page's route is
      built.
    - `pane.open` accepts `browser:identity` in `meta`, validated: a known profile id or
      `incognito`.
  - **Restore:** a replayed layout's Incognito tabs get fresh `incognito:<jar>` values
    (`browser_identity::refresh_for_replay`).
  - **Fixed once set:** client writes to `browser:identity` are refused, like the other browser
    keys srv guards. A tab's identity is given only when it is opened (`pane.open`, checked by
    `browser_identity::check_new_tab`), and a popup pane inherits its opener's.
- **Host:**
  - **Contexts:** `browser_pane_create` gets the identity. `resolve_pane_request_context`
    (identities spec §4.1) gives:
    - **Personal:** the global context, as today;
    - **Incognito:** a context with an empty `cache_path`, kept in a per-block map until the
      tab closes;
    - **A profile:** a disk-backed context at `<cef-cache>/profile-<id>`, one per profile,
      shared by its tabs.
  - **Popup contexts:** a popup gets its opener's context.
  - **Writing sign-ins (P2):** the host exits without shutting CEF down, so cookies newer than
    Chromium's last periodic write (about every 30 seconds) were lost on quit. A profile's cookie
    store is written when one of its tabs finishes loading a page, which is how a sign-in ends.
- **Frontend:**
  - **Toolbar and menu:** the Home and Profile buttons in `browser-nav-bar.tsx`, the menu as a
    `FlyoutMenu`, and the inline New-profile form.
  - **Opening tabs:** a new tab is `openBlockInStack(layout, {blockId}, "browser", {view:
    "browser", url, "browser:identity": …})`.
  - **Pill badge:** a small badge slot added to the pane-tab pill, reused by any view.
  - **Settings:** the Profiles settings section.

## 7. What stands in the way

1. **Named profiles need a jar on disk, and CEF hangs on one today** (identities spec §1.5 A).
   A context whose `cache_path` is a direct child of the cache root makes Chrome build a real
   profile, and the browser is then never created. The hypotheses (from that spec's Phase 3):
   - the profile is created asynchronously while the browser is created at once;
   - the window-creation runner may be what wedges it;
   - it was only ever seen for top-level windows, never for the Windows child-pane path.
   **So a one-day spike comes first:** create the profile context ahead of time, wait for it to
   be ready (a callback-taking call such as getting its cookie manager), then create the pane
   in it.
2. **Linux and macOS: a pane whose jar differs from its window's crashes CEF** (identities spec
   §1.5 B). Windows is unaffected. On those platforms, until the CEF patch (that spec's §4.4
   route 1) lands, Incognito and other profiles open in a **floating pane**: their own small
   window, created with that jar (route 2). The menu says so ("opens in its own window here").
3. **Memory:** every jar is its own Chrome profile, with at least one more renderer process.
   - **Incognito:** up to 8 tabs at once (the identities spec's proposal); past that, the menu
     item says why it can't open another.
   - **Profiles:** cost one jar each, however many tabs they have.

## 8. Phases

| | Scope | Ships |
|---|---|---|
| **P1** | Home button, button order, "Home page" naming; the Profile button and menu with Personal and **Open new Incognito tab** (Windows); badges; Incognito tabs back signed out | The toolbar and Incognito, with nothing blocked |
| **P2** | The spike on disk-backed profiles (§7.1), then **New profile…**, the profile list, Manage profiles, restore signed in | Named profiles, once the spike works |
| **P3** | Linux/macOS: floating-pane fallback, then the CEF patch | The same menu on every platform |
| **P4** | `OpenBrowser({profile})` and "Agents may use this profile" | Agents in their own jars |

Until P2, the menu shows Personal, Open new Incognito tab, and a disabled **New profile…** with
"Coming soon", so the menu's shape doesn't change when profiles arrive.

## 9. Decisions (and what to revisit)

| Question | Decision |
|---|---|
| Where Incognito opens | A new tab in this pane, like a profile (the ask's "window pane"). A floating window only where the platform needs it (§7.2). |
| What a new tab opens | The current page, so "this site as another account" is one click; Home for a new profile, and from a blank page. |
| Switching a tab's identity | Not possible: open a new tab. No reload-and-lose-your-page step. |
| Name | "Incognito", as asked: the word people know. The header line says what it does and doesn't keep. |
| Bookmarks and Home | Global, shared by every profile. Per-profile bookmarks can come later if wanted. |
| Incognito after restart | Back signed out, with a fresh jar: its pane is in srv's store, its jar was only in memory. |
| The cap | 8 Incognito tabs; P1 measures the memory and sets it. |
| Agents and profiles | Off for each profile until the user switches on "Agents may use it" in Settings → Browser. Incognito is always allowed. Agents can list profiles but never change them. |

## 10. Tests

- **Frontend (vitest):**
  - the toolbar order;
  - Home goes to the start page, else the default;
  - the menu lists Personal with ✓ and Incognito, and (P2) the profiles;
  - choosing an entry opens a tab in the same pane with the right `browser:identity` and
    address;
  - the badges render from meta.
- **srv:**
  - `pane.open` validates `browser:identity`;
  - a replayed layout gives Incognito tabs fresh jars;
  - (P2) the registry's create, rename and delete, and delete closing that profile's tabs.
- **Host:**
  - the context registry's create, reuse and drop;
  - a popup gets its opener's context.
- **Live (`task dev`, Windows):**
  - two tabs on one site signed in to two accounts, one Personal and one Incognito;
  - an Incognito tab's sign-in popup signs in inside it;
  - closing the tab forgets the sign-in;
  - after a restart it is back, signed out;
  - (P2) a Work profile stays signed in across a restart.
