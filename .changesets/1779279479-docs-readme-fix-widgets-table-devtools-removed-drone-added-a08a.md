---
type: patch
---

docs(readme): fix Widgets table — DevTools removed, Drone added

The README's Widgets table drifted from `widgets.json`:

- Removed the **DevTools** widget row — DevTools stopped being a widget in
  PR #936; it's now a hamburger-menu item. Added a corresponding row to the
  "Not widgets — opened from elsewhere" table (Hamburger ≡ → DevTools).
- Added the **Drone** widget row (`diagram-project` icon, More tier) — it
  was present in `widgets.json` but missing from the README table.
