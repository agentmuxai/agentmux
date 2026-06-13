(() => {
  const term = document.querySelector('.xterm');
  if (!term) return JSON.stringify({ error: 'no terminal' });
  const connect = term.closest('.term-connectelem') || document.querySelector('.term-connectelem');
  const screen = term.querySelector('.xterm-screen');
  const scrollable = term.querySelector('.xterm-scrollable-element');
  // Walk up to the block content frame that paints the background behind the gap.
  const blockInner = term.closest('.block-frame-default-inner') || term.closest('[class*="block-content"]') || connect?.parentElement;

  const box = (el, label) => {
    if (!el) return { label, missing: true };
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return {
      label,
      cls: el.className?.toString?.().slice(0, 40),
      x: +r.x.toFixed(1), y: +r.y.toFixed(1), w: +r.width.toFixed(1), h: +r.height.toFixed(1),
      right: +r.right.toFixed(1), bottom: +r.bottom.toFixed(1),
      margin: cs.margin, padding: cs.padding,
      bg: cs.backgroundColor,
    };
  };

  // cell size from xterm core
  const core = term.__core || null;
  let cell = null, cols = null, rows = null;
  try {
    // reach via a known global if present, else skip
    const anyTerm = window.__lastTerm;
    if (anyTerm?._core) {
      const d = anyTerm._core._renderService.dimensions;
      cell = { w: +d.css.cell.width.toFixed(2), h: +d.css.cell.height.toFixed(2) };
      cols = anyTerm.cols; rows = anyTerm.rows;
    }
  } catch (e) {}

  const connectR = connect?.getBoundingClientRect();
  const screenR = screen?.getBoundingClientRect();
  const blockR = blockInner?.getBoundingClientRect();

  // Gaps between the text (screen) and the connect element edges, and between
  // connect element and the block frame (margins).
  const gaps = {};
  if (connectR && screenR) {
    gaps.screen_to_connect = {
      left: +(screenR.left - connectR.left).toFixed(1),
      right: +(connectR.right - screenR.right).toFixed(1),
      top: +(screenR.top - connectR.top).toFixed(1),
      bottom: +(connectR.bottom - screenR.bottom).toFixed(1),
    };
  }
  if (blockR && connectR) {
    gaps.connect_to_block = {
      left: +(connectR.left - blockR.left).toFixed(1),
      right: +(blockR.right - connectR.right).toFixed(1),
      top: +(connectR.top - blockR.top).toFixed(1),
      bottom: +(blockR.bottom - connectR.bottom).toFixed(1),
    };
  }

  return JSON.stringify({
    dpr: window.devicePixelRatio,
    cell, cols, rows,
    layers: [
      box(blockInner, 'block-inner'),
      box(connect, 'term-connectelem'),
      box(term, 'xterm'),
      box(scrollable, 'scrollable-element'),
      box(screen, 'xterm-screen'),
    ],
    gaps,
  }, null, 2);
})()
