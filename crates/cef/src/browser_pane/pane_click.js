// Linux only. Runs in the isolated world of each document in a browser pane, next to side_button.js (the host
// creates the world and evaluates this in it, see side_button_cdp.rs). A button going down in the page tells the
// host, through the binding the host added to this world, that the person clicked in this pane: the host selects
// the pane and tells the window's page, which closes the menus and popovers over it.
//
// The page cannot fake it. The binding exists only in this world, which the page's scripts cannot reach, and it
// carries nothing: the host names the pane from the browser the call came from. A click a page's own script makes
// (`dispatchEvent`, `el.click()`) has `isTrusted` false and is ignored here.
(() => {
  if (window.__agentmuxPaneClick) return;
  window.__agentmuxPaneClick = true;
  window.addEventListener(
    "pointerdown",
    (e) => {
      // The primary, middle and secondary buttons, and a touch or a pen (button 0). The side buttons (3, 4) are
      // the history buttons: side_button.js takes those.
      if (!e.isTrusted || e.button < 0 || e.button > 2) return;
      // The host adds the binding to this world; until it has, the click is only the page's.
      if (typeof window.agentmuxPaneClick === "function") window.agentmuxPaneClick("");
    },
    { capture: true, passive: true },
  );
})();
