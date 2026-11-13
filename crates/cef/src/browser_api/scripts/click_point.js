function () {
  // Where to click the element (`this`), in the page's CSS pixels, after scrolling it into view; or why a click
  // can't land (docs/specs/SPEC_BROWSER_PANE_AGENT_ZOOM_2026_11_10.md §4). Run in the element's own frame.
  const doc = this.ownerDocument;
  const win = doc.defaultView;
  const hint = " (try Browser zoom: fit, or factor 0.5)";
  // A checkbox or radio the page hides (display: none) behind a label drawn
  // as the box has no box of its own: click its label, at a point where the
  // label itself is hit, not a link or button inside it.
  const choice = this.tagName === "INPUT" && /^(checkbox|radio)$/i.test(this.type);
  if (choice && this.getClientRects().length === 0) {
    const label = Array.from(this.labels || []).find(l => l.getClientRects().length > 0);
    if (!label) return { error: "the element is not visible" };
    label.scrollIntoView({ block: "center", inline: "center" });
    const clear = (x, y) => {
      const at = doc.elementFromPoint(x, y);
      if (!at || !label.contains(at)) return false;
      for (let n = at; n && n !== label; n = n.parentElement) {
        if (n.matches("a[href], button, select, textarea, input, [role=button], [role=link]")) return false;
      }
      return true;
    };
    for (const el of [label, ...label.querySelectorAll("*")]) {
      const r = el.getBoundingClientRect();
      if (r.width <= 0 || r.height <= 0) continue;
      const x = r.left + r.width / 2, y = r.top + r.height / 2;
      if (clear(x, y)) return { x, y };
    }
    return { error: "the label of this checkbox is covered by something else; ask the user to tick it" };
  }
  this.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  const rect = this.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) return { error: "the element is not visible" };
  const vw = win.innerWidth, vh = win.innerHeight;
  // No viewport to measure against (a frame with no size yet): the centre, as before.
  if (!(vw > 0 && vh > 0)) return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  // A line or a sub-pixel box is a target one pixel wide or tall, so being inside the view is what counts.
  const grow = (lo, hi) => (hi - lo >= 1 ? [lo, hi] : [(lo + hi) / 2 - 0.5, (lo + hi) / 2 + 0.5]);
  const [left, right] = grow(rect.left, rect.right);
  const [top, bottom] = grow(rect.top, rect.bottom);
  const r = { left, right, top, bottom };

  // The part a click can reach: inside the viewport and inside every ancestor that clips it.
  const cut = (box, c, byX, byY) => ({
    left: byX ? Math.max(box.left, c.left) : box.left,
    right: byX ? Math.min(box.right, c.right) : box.right,
    top: byY ? Math.max(box.top, c.top) : box.top,
    bottom: byY ? Math.min(box.bottom, c.bottom) : box.bottom,
  });
  const empty = b => b.right <= b.left || b.bottom <= b.top;
  const inView = cut(r, { left: 0, top: 0, right: vw, bottom: vh }, true, true);
  let box = inView;
  let clipper = null;
  // What clips an element depends on how it is placed: a fixed one is clipped by no ancestor (unless one
  // transforms), an absolute one only by its containing block and what is outside it. The page's own root
  // and body are the viewport, which is already applied.
  let placed = win.getComputedStyle(this).position;
  for (let p = this.parentElement; p && p !== doc.body && p !== doc.documentElement && !empty(box); p = p.parentElement) {
    const s = win.getComputedStyle(p);
    const makesBlock = s.transform !== "none" || s.perspective !== "none" || s.filter !== "none";
    if (placed === "fixed" && !makesBlock) continue;
    if (placed === "absolute" && s.position === "static" && !makesBlock) continue;
    placed = s.position;
    const byX = s.overflowX !== "visible", byY = s.overflowY !== "visible";
    if (!byX && !byY) continue;
    const next = cut(box, p.getBoundingClientRect(), byX, byY);
    if (empty(next)) clipper = p;
    box = next;
  }

  // The page's own words in an answer are the host's to mask (a secret the page shows once) and shorten:
  // they leave here whole, between \u0001 and \u0002 (`tidy_click_error` in act.rs).
  const name = el => {
    const text = ((el.getAttribute && el.getAttribute("aria-label")) || el.innerText || el.textContent || "").trim().replace(/\s+/g, " ");
    return "<" + el.tagName.toLowerCase() + (text ? ' "\u0001' + text.slice(0, 400) + '\u0002"' : "") + ">";
  };
  if (empty(box)) {
    if (clipper) return { error: "not clicked: " + name(this) + " is cut off by " + name(clipper) + ", which clips it and can't be scrolled to it" + hint };
    const dx = r.right <= 0 ? -r.right : r.left >= vw ? r.left - vw : 0;
    const dy = r.bottom <= 0 ? -r.bottom : r.top >= vh ? r.top - vh : 0;
    return { error: "not clicked: " + name(this) + " is outside the visible area by " + Math.round(Math.max(dx, dy)) + " px" + hint };
  }

  // Is the thing a click lands on the target, or part of it (or what it is part of)?
  const within = (n, anc) => {
    for (let x = n; x; x = x.parentNode || x.host) if (x === anc) return true;
    return false;
  };
  const labels = Array.from(this.labels || []);
  const related = at => within(at, this) || within(this, at) || labels.some(l => within(at, l));
  const hit = (x, y) => {
    let at = doc.elementFromPoint(x, y);
    while (at && at.shadowRoot) {
      const inner = at.shadowRoot.elementFromPoint(x, y);
      if (!inner || inner === at) break;
      at = inner;
    }
    return at;
  };
  const cx = (box.left + box.right) / 2, cy = (box.top + box.bottom) / 2;
  const dw = (box.right - box.left) / 4, dh = (box.bottom - box.top) / 4;
  const points = [[cx, cy], [cx - dw, cy - dh], [cx + dw, cy - dh], [cx - dw, cy + dh], [cx + dw, cy + dh]];
  let covering = null;
  for (const [x, y] of points) {
    const at = hit(x, y);
    if (at && related(at)) return { x, y };
    if (at && !covering) covering = at;
  }
  if (!covering) return { error: "not clicked: nothing at " + name(this) + " takes a click" + hint };
  return { error: "not clicked: " + name(this) + " is covered by " + name(covering) + hint };
}
