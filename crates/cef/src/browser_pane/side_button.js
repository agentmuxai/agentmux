// Runs in an isolated world of each document in a browser pane (the host
// creates the world and evaluates this in it, see side_button_cdp.rs). The mouse
// side buttons (`button` 3 = Back, 4 = Forward) are claimed here: Chromium
// doesn't navigate the page on them, and a page listener registered after this
// one doesn't see them. The release reports to the host through the binding the
// host added to this world; the host decides what it means (the window's
// selection history, or, with Ctrl, the page's own history), as for a browser
// pane on Windows.
(() => {
  if (window.__agentmuxSideButtons) return;
  window.__agentmuxSideButtons = true;
  const DEBOUNCE_MS = 150;
  let last = { button: null, at: 0 };
  const handler = (e) => {
    if (!e.isTrusted || (e.button !== 3 && e.button !== 4)) return;
    // Cancelling the pointerdown makes Chromium drop the mouse events after it;
    // a chorded press (another button held) has no pointerdown, so its mouse
    // events are claimed too. Both releases can arrive: the debounce makes one.
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.type !== "pointerup" && e.type !== "mouseup") return;
    const button = e.button === 3 ? "back" : "forward";
    const now = performance.now();
    if (last.button === button && now - last.at < DEBOUNCE_MS) return;
    last = { button, at: now };
    const blocked = e.shiftKey || e.altKey || e.metaKey || (e.buttons & 1) === 1;
    // The host adds the binding to this world; until it has, the press is only claimed.
    if (typeof window.agentmuxSideButton === "function") {
      window.agentmuxSideButton(JSON.stringify({ button, ctrl: e.ctrlKey, blocked }));
    }
  };
  for (const type of ["pointerdown", "pointerup", "mousedown", "mouseup", "auxclick"]) {
    window.addEventListener(type, handler, { capture: true, passive: false });
  }
})();
