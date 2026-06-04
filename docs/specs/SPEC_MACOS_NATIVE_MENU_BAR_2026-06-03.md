# SPEC: Native macOS menu bar (File / Edit / View / Window / Help)

**Status:** Draft · **Date:** 2026-06-03 · **Owner:** AgentMux host/macOS
**Related:** `SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03.md` (the menu bar appeared because the app now runs as a regular Dock app), `frontend/app/window/hamburger-menu.tsx`, `frontend/app/store/keymodel.ts`, `frontend/app/menu/base-menus.ts`

---

## 0. Motivation

AgentMux now runs as a **regular, Dock-visible macOS app** (`set_macos_activation_policy_regular`, `main.rs`). That gives every window the system **menu bar** at the top of the screen — currently near-empty (just the default app menu macOS synthesizes). macOS users expect a populated, HIG-standard menu bar; the empty slots read as unfinished.

This is also a real usability win, not just polish:

- A native **Edit** menu with the standard Cut/Copy/Paste/Undo/Redo/Select-All items (wired to AppKit's first-responder selectors) makes those shortcuts work **reliably in the web text fields** and exposes them to the OS (Services, dictation, the menu's own discoverability). Today they depend entirely on Chromium's internal handling.
- The actions buried in the title-bar **hamburger** and **Settings** become **discoverable** in the places macOS users look for them (File ▸ New Window, View ▸ Theme, Help ▸ Docs, AgentMux ▸ Settings…).
- Standard **Window** and **App** menus give Minimize/Zoom/Hide/Quit/About their conventional homes and shortcuts.

Goal: a complete, HIG-faithful menu bar on macOS that **shares one set of action handlers** with the hamburger and keybindings — no divergent logic, no double-fired shortcuts. Windows/Linux are unaffected (they keep the hamburger).

---

## 1. Principles

1. **HIG-faithful.** Standard menus, ordering, item names, and key equivalents per Apple's *Human Interface Guidelines → Menus* and the standard menu-bar layout.
2. **One source of truth for actions.** A native menu item, the hamburger item, and the keybinding for the same action all call the **same** frontend handler. Factor a shared `MenuAction` registry.
3. **Standard items use standard selectors.** Cut/Copy/Paste/Undo/Redo/Select All/Minimize/Zoom/Hide/Quit route through AppKit's standard selectors to the first responder, so the focused web view (or native control) handles them correctly. This is the load-bearing reason for a native Edit menu.
4. **One owner per shortcut.** A key equivalent is handled by exactly one layer. On macOS, key equivalents that live on a native menu item are **ceded by `keymodel.ts`** to avoid double-firing.
5. **macOS-only.** The native menu bar is built only on macOS; the hamburger stays the affordance on Windows/Linux. No behavior change off-macOS.

---

## 2. Current inventory

### 2.1 Hamburger menu (`hamburger-menu.tsx`)
| Item | Action | Shortcut today |
|------|--------|----------------|
| New Tab | `createTab()` | ⌘T |
| New Window | `getApi().openNewWindow()` | ⌘⇧N |
| Theme ▸ | set `window:theme` | — |
| Opacity ▸ | set `window:opacity`/`transparent` | — |
| Settings | open `settings.json` in editor | — |
| Command Palette | `openModal(CommandPaletteModal)` | ⌘P |
| Identity & Memory | `openBundleManager()` | — |
| DevTools | `getApi().toggleDevtools()` | — |
| Online Docs | open `https://docs.agentmux.ai` | — |
| Exit | `close_window` | — |

### 2.2 Global keybindings (`keymodel.ts`)
| Shortcut | Action |
|----------|--------|
| ⌘T | New Tab (`createTab`) |
| ⌘N | New Pane (`createBlock(default)`) |
| ⌘⇧N (⌃⇧N) | New Window |
| ⌘W | Close (tab/pane) |
| ⌘⇧W | Close static tab |
| ⌘D / ⌘⇧D | Split right / Split down |
| ⌘[ / ⌘] | Previous / Next tab |
| ⌘1…⌘9 | Switch to tab N |
| ⌃[ / ⌃] | Cycle pane focus |
| ⌃⇧↑/↓/←/→ | Navigate pane in direction |
| ⌘M | **Magnify focused pane** (collides with standard Minimize) |
| ⌘F | Find / search in pane |
| ⌘= / ⌘+ / ⌘- / ⌘0 | Zoom in / in / out / reset |
| ⌘I | Refocus (`globalRefocus`) |
| ⌘G | Switch connection |
| ⌃⇧M | Toggle terminal multi-input |
| ⌃⇧V | Toggle voice input |
| ⌃⇧K | Open launcher block |

---

## 3. Proposed menu scheme

Legend: **[std]** = standard AppKit selector (first responder); **[fe]** = dispatches to a frontend handler; **[host]** = host-side.

### AgentMux (app menu)
| Item | Shortcut | Source |
|------|----------|--------|
| About AgentMux | | [fe] about modal (`getAboutModalDetails`) |
| — | | |
| Settings… | ⌘, | [fe] open `settings.json` (was hamburger "Settings"; standardize to ⌘,) |
| — | | |
| Identity & Memory… | | [fe] `openBundleManager()` |
| — | | |
| Services | | [std] |
| — | | |
| Hide AgentMux | ⌘H | [std] `hide:` |
| Hide Others | ⌥⌘H | [std] `hideOtherApplications:` |
| Show All | | [std] `unhideAllApplications:` |
| — | | |
| Quit AgentMux | ⌘Q | [fe/host] graceful `close_window` → quit |

### File
| Item | Shortcut | Source |
|------|----------|--------|
| New Tab | ⌘T | [fe] `createTab` |
| New Pane | ⌘N | [fe] `createBlock(default)` |
| New Window | ⌘⇧N | [fe] `openNewWindow` |
| — | | |
| Close Tab | ⌘W | [fe] `genericClose` |
| Close Window | ⌘⇧W | [fe] close window |

### Edit  *(all standard selectors — makes editing work in web inputs)*
| Item | Shortcut | Source |
|------|----------|--------|
| Undo | ⌘Z | [std] `undo:` |
| Redo | ⌘⇧Z | [std] `redo:` |
| — | | |
| Cut | ⌘X | [std] `cut:` |
| Copy | ⌘C | [std] `copy:` |
| Paste | ⌘V | [std] `paste:` |
| Paste and Match Style | ⌥⌘⇧V | [std] `pasteAsPlainText:` |
| Delete | ⌫ | [std] `delete:` |
| Select All | ⌘A | [std] `selectAll:` |
| — | | |
| Find ▸ Find… | ⌘F | [fe] `activateSearch` (or [std] `performTextFinderAction:`) |
| Speech / Emoji & Symbols | | [std] (optional) |

### View
| Item | Shortcut | Source |
|------|----------|--------|
| Command Palette… | ⌘P | [fe] `openModal(CommandPaletteModal)` |
| — | | |
| Theme ▸ | | [fe] dynamic submenu, checkmark on active |
| Opacity ▸ | | [fe] dynamic submenu, checkmark on active |
| — | | |
| Zoom In | ⌘+ | [fe] zoom in |
| Zoom Out | ⌘- | [fe] zoom out |
| Actual Size | ⌘0 | [fe] zoom reset |
| — | | |
| Enter Full Screen | ⌃⌘F | [std] `toggleFullScreen:` |
| — | | |
| Developer ▸ Toggle DevTools | ⌥⌘I | [fe] `toggleDevtools` |

### Pane  *(AgentMux's pane/layout model — custom menu)*
| Item | Shortcut | Source |
|------|----------|--------|
| Split Right | ⌘D | [fe] `handleSplitHorizontal` |
| Split Down | ⌘⇧D | [fe] `handleSplitVertical` |
| — | | |
| Focus Next / Previous Pane | ⌃] / ⌃[ | [fe] `cyclePaneFocus` |
| Focus Pane Up/Down/Left/Right | ⌃⇧↑↓←→ | [fe] `switchBlockInDirection` |
| Magnify Pane | ⌃⌘M *(remapped — see §4)* | [fe] `magnifyNodeToggle` |
| — | | |
| Terminal Multi-Input | ⌃⇧M | [fe] toggle multi-input |
| Voice Input | ⌃⇧V | [fe] toggle voice |

### Window  *(standard)*
| Item | Shortcut | Source |
|------|----------|--------|
| Minimize | ⌘M | [std] `performMiniaturize:` |
| Zoom | | [std] `performZoom:` |
| — | | |
| Next Tab | ⌘] | [fe] `switchTab(1)` |
| Previous Tab | ⌘[ | [fe] `switchTab(-1)` |
| — | | |
| Bring All to Front | | [std] |
| *(window list)* | | [std] |

### Help
| Item | Shortcut | Source |
|------|----------|--------|
| AgentMux Help / Online Docs | | [fe] open `https://docs.agentmux.ai` |
| Keyboard Shortcuts | | [fe] (optional — shortcuts reference) |
| *(Search)* | | [std] |

---

## 4. Shortcut reconciliation (the critical part)

1. **⌘M conflict.** Today ⌘M = *magnify pane*; macOS reserves ⌘M = *Minimize*. The native **Window ▸ Minimize ⌘M** must win (HIG). **Remap magnify** to `⌃⌘M` (open question §9) and update `keymodel.ts` + any docs/onboarding.
2. **One owner per key on macOS.** For every item that carries a key equivalent in the native menu, **`keymodel.ts` must cede that chord on macOS** (guard the `globalKeyMap.set` or early-return) so the action fires once. The native item's action still dispatches to the same frontend handler, so behavior is identical — only the dispatch path changes.
   - Standard-selector items (Cut/Copy/Paste/Undo/Select All/Minimize/Zoom/Full Screen) should be ceded entirely to AppKit; remove any JS interception of them on macOS.
   - App shortcuts (⌘T/⌘N/⌘⇧N/⌘W/⌘⇧W/⌘P/⌘D/⌘⇧D/⌘±/⌘0/⌘F/⌘[/⌘]) live on native items; keymodel cedes them on macOS.
   - Pane/utility chords without a menu key equivalent (⌃] ⌃[ ⌃⇧arrows ⌃⇧M ⌃⇧V ⌘1–9 ⌘G ⌘I) **stay in keymodel** unchanged.
3. **No silent doubles.** A short test matrix asserts each chord fires its action exactly once on macOS.

---

## 5. The hamburger on macOS

With a complete native menu bar, the title-bar hamburger is **redundant on macOS**. Options:

- **(A — recommended)** Remove the hamburger on macOS; its items now live in the menu bar. Cleanest, most HIG-native. (Windows/Linux keep it unchanged — it's their primary affordance.)
- **(B)** Keep a slim "quick actions" hamburger (e.g., New Tab, Command Palette, Theme) for mouse-first users who don't want to travel to the top menu bar.

Recommend **A**, revisit if users miss quick title-bar access. Either way the far-right placement work from the mirrored-menu PR stays correct for non-macOS.

---

## 6. Dynamic items

- **Theme / Opacity** submenus carry a checkmark on the active value; rebuild or revalidate on menu-open via an `NSMenuDelegate` (`menuNeedsUpdate:`) reading current `settings`.
- **Enable/disable** (validation): items like Close Tab, Split, Magnify depend on focus/state. Use `validateMenuItem:` (or a per-open refresh) querying the frontend for current capability; default-enable in Phase 1.
- **Window list / tab list**: standard Window-menu window list is automatic; an optional tab list can be populated on open.

---

## 7. Implementation

**Where:** the host builds the `NSMenu` tree and calls `[NSApp setMainMenu:]` after `cef::initialize` (the app exists then — same hook point as `set_macos_activation_policy_regular`). Build it via raw libobjc FFI (mirroring the existing `main.rs` swizzles) or a small compiled `.m` helper if the tree gets large.

**Standard items** are created with their AppKit selectors (`cut:`, `copy:`, `paste:`, `undo:`, `redo:`, `selectAll:`, `performMiniaturize:`, `performZoom:`, `toggleFullScreen:`, `hide:`, `hideOtherApplications:`, `unhideAllApplications:`, `terminate:`) and `nil` target → AppKit routes them to the first responder (the focused CEF web view). No app wiring needed; this is what makes web-input editing robust.

**Custom items** ([fe]) use a single host-side target object whose action posts an IPC event — `menu:invoke { commandId }` — to the **key window's** frontend. The frontend has a `MenuAction` registry keyed by `commandId` that runs the same handler the hamburger/keymodel already call. Refactor so all three (native menu, hamburger, command palette) resolve through that one registry — the single source of truth from §1.

**Key equivalents:** set on native items (`setKeyEquivalent:` + modifier mask). Combined with §4's keymodel cession, each chord has one owner. macOS renders the shortcut next to the item automatically.

**App-global vs per-window:** the menu bar is app-global; [fe] actions target the key window's renderer, so they operate on the window the user is actually in.

**Strings & About:** menu titles centralized for future localization; About uses `getAboutModalDetails()` (version/build label already available).

---

## 8. Phasing

- **Phase 1 (immediate win):** static menu bar — App / File / Edit / View / Window / Help. All **standard** editing + window items (makes ⌘C/⌘V/⌘Z/⌘A/Minimize robust on day one) + core custom items (New Tab/Pane/Window, Settings…, Command Palette…, Toggle DevTools, Online Docs, About, Quit). Wire the `MenuAction` registry + `menu:invoke` IPC. Cede the corresponding keymodel chords on macOS.
- **Phase 2:** dynamic Theme/Opacity submenus with checkmarks; the **Pane** menu (split/focus/magnify/multi-input/voice); zoom items; `validateMenuItem:`; ⌘M→⌃⌘M magnify remap.
- **Phase 3:** remove/trim the hamburger on macOS (§5A); finalize shortcut reconciliation + the once-only test matrix; Window/tab list niceties.

---

## 9. Open questions

- **Magnify remap target** — `⌃⌘M` proposed; confirm no other conflict.
- **Hamburger fate on macOS** — remove (A) vs slim quick-actions (B).
- **Identity & Memory** placement — App menu vs a custom menu vs Window.
- **Quit semantics** — must run the same graceful shutdown as the current Exit/`close_window` (flush state, srv teardown), not a bare `terminate:`.
- **Per-window menu validation** — how much state to query on each menu-open vs cache.

---

## 10. References

- Apple HIG — *Menus* and the standard macOS menu-bar layout (App, File, Edit, Format, View, Window, Help) and standard key equivalents.
- Internal: `frontend/app/window/hamburger-menu.tsx` (current items), `frontend/app/store/keymodel.ts` (shortcuts/actions), `frontend/app/menu/base-menus.ts` (shared menu builder), `frontend/app/modals/command-palette.tsx`, `agentmux-cef/src/main.rs` (`set_macos_activation_policy_regular`, the swizzle/FFI patterns to mirror for `setMainMenu:`), `SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03.md`.
