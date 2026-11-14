function () {
  // Before a click on the element (`this`): is its page being shown, and start counting the mouse events that
  // reach its window, so the host can tell a click the page took from one it never received (a page that is not
  // shown, say a minimized window, is sent the events and does nothing with them). Run in the element's own frame,
  // in the isolated world: the page's scripts can't see this window object's extra property.
  const doc = this.ownerDocument;
  const win = doc.defaultView;
  if (doc.visibilityState === "hidden") return { hidden: true };
  if (win.__amxClickWatch) win.__amxClickWatch.stop();
  const types = ["pointerdown", "mousedown", "mouseup", "click"];
  const watch = { seen: 0 };
  const on = () => { watch.seen += 1; };
  for (const t of types) win.addEventListener(t, on, true);
  watch.stop = () => { for (const t of types) win.removeEventListener(t, on, true); };
  win.__amxClickWatch = watch;
  return { hidden: false };
}
