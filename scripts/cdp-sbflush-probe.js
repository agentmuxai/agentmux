(() => {
  const term = document.querySelector('.xterm');
  if (!term) return JSON.stringify({ error: 'no terminal' });
  const connect = term.closest('.term-connectelem') || document.querySelector('.term-connectelem');
  const m = (el, label) => {
    if (!el) return { label, missing: true };
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return {
      label,
      right: +r.right.toFixed(1), w: +r.width.toFixed(1),
      cssWidth: cs.width, display: cs.display, position: cs.position,
    };
  };
  const connectR = connect?.getBoundingClientRect();
  const vbar = term.querySelector('.scrollbar.vertical');
  const vbarR = vbar?.getBoundingClientRect();
  return JSON.stringify({
    connectRight: connectR ? +connectR.right.toFixed(1) : null,
    connectW: connectR ? +connectR.width.toFixed(1) : null,
    layers: [
      m(connect, 'connect'),
      m(term, 'xterm'),
      m(term.querySelector('.xterm-viewport'), 'viewport'),
      m(term.querySelector('.xterm-scrollable-element'), 'scrollable'),
      m(term.querySelector('.xterm-screen'), 'screen'),
      m(vbar, 'scrollbar-vertical'),
    ],
    // gap from scrollbar right edge to the connect (pane) right edge:
    scrollbar_to_paneRight: (vbarR && connectR) ? +(connectR.right - vbarR.right).toFixed(1) : null,
  }, null, 2);
})()
