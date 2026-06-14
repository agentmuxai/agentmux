---
type: patch
---

fix(term): xterm-6 terminal scrollbar is clickable and shows the default cursor — lift the overlay scrollbar above the link-layer canvas (z-index) and force cursor:default; retire the dead xterm-5 fit/reservation + native-scrollbar code and add a CDP hit-test smoke guard (#1369, #1370)
