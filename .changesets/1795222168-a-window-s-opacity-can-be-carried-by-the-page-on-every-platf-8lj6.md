---
type: patch
---

Menus, submenus, popovers and drop-downs stay fully solid at any window opacity on Windows, macOS and Linux: the window opacity is carried by the page's own backgrounds instead of a whole-window alpha, and floating panels paint the opaque menu fill. Text keeps its sharp sub-pixel smoothing at 100% opacity: the page carries the opacity from the start only after a window has been below 100%, and a window lowered in a session that started at 100% fades with the system until the next start, which the Opacity menu says
