function (last) {
  // After a click on the element (`this`): how many mouse events reached its window since click_watch.js ran.
  // `null` when no watch is running (the page was replaced). The watch ends once something was seen, or when
  // `last` says this is the final look.
  const win = this.ownerDocument.defaultView;
  const watch = win.__amxClickWatch;
  if (!watch) return { seen: null };
  const seen = watch.seen;
  if (seen > 0 || last) {
    watch.stop();
    delete win.__amxClickWatch;
  }
  return { seen };
}
