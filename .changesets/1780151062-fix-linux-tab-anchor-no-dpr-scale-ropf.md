---
type: patch
---

fix(linux): tab tear-off anchor — drop DPR scale now that screenX is DIP

reagent P2 against `edda8911` on PR #1188: the tab-anchor in
`.linux.tsx` does `screenX - grabOffset.x * dpr`, which was correct
when `screenX` came from `get_cursor_point` (Windows-style
`GetCursorPos` returning physical px). The prior commit on this PR
switched `screenX/Y` to DOM `e.screenX/Y` (CSS px = DIP) to fix the
floater-at-screen-origin bug — but the tab-anchor multiplier still
assumed physical px. On HiDPI Linux that double-scales the grab
offset, placing the tab tear-off window off by the grab-offset
amount; harmless only at dpr=1.

Fix: drop the `* dpr` on both axes. Both `screenX/Y` and
`grabOffset.x/y` are now DIP, and CEF Views positions in DIP on
Linux, so plain subtraction is correct. The `* dpr` survives in the
win32 sibling, where it's still right (`get_cursor_point` returns
physical px there).

Updated the inline comment to spell out the DIP arithmetic and why
the win32 sibling diverges.
