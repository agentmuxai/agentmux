// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// DOM query helper injected into a browser-pane context via CDP
// `Runtime.evaluate`. Called by /agentmux/browser/query.
//
// Idempotent: re-injected on every request, but the IIFE checks
// for an existing definition before replacing, so visible state on
// window is stable across calls.

(() => {
  if (window.__amq_query) return;

  // Compute a :nth-of-type path from <body> to `el` — stable enough
  // to use as a follow-up targeting handle, as long as the DOM
  // hasn't been restructured in between.
  window.__amq_path = (el) => {
    if (!el || el.nodeType !== 1) return '';
    if (el === document.body) return 'body';
    const segs = [];
    let node = el;
    while (node && node.nodeType === 1 && node !== document.body) {
      const tag = node.tagName.toLowerCase();
      const parent = node.parentNode;
      if (!parent) { segs.unshift(tag); break; }
      // nth-of-type among siblings of same tag
      let idx = 1;
      for (const sib of parent.children) {
        if (sib === node) break;
        if (sib.tagName === node.tagName) idx += 1;
      }
      segs.unshift(`${tag}:nth-of-type(${idx})`);
      node = parent;
    }
    segs.unshift('body');
    return segs.join(' > ');
  };

  // Snapshot whatever has focus right now as our Element shape.
  // Returns null when nothing meaningful is focused (document.body
  // is the default resting state; there's no input focus to report).
  window.__amq_focus_info = () => {
    const el = document.activeElement;
    if (!el || el === document.body) return null;
    const r = el.getBoundingClientRect();
    const attrs = {};
    for (const a of el.attributes) attrs[a.name] = a.value;
    return {
      selector: window.__amq_path(el),
      tag: el.tagName.toLowerCase(),
      text: (el.textContent || '').slice(0, 500),
      attrs,
      rect: { x: r.x, y: r.y, width: r.width, height: r.height },
      focused: true,
    };
  };

  // Return viewport-space centroid of the first selector match, or
  // null if no element matches. Used by /agentmux/browser/click_element
  // to convert a CSS selector into Input.dispatchMouseEvent coords.
  window.__amq_centroid_of = (selector) => {
    let el;
    try { el = document.querySelector(selector); } catch (e) { return null; }
    if (!el) return null;
    // Scroll into view so the centroid is actually clickable. Chromium
    // silently drops dispatchMouseEvent events that land outside the
    // visible viewport; without the scroll, an off-screen selector
    // would look like a successful call that does nothing.
    el.scrollIntoView({ block: 'center', inline: 'center' });
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  };

  window.__amq_query = (selector, limit) => {
    let nodes;
    try {
      nodes = document.querySelectorAll(selector);
    } catch (e) {
      return { error: String(e) };
    }
    const out = [];
    const n = limit > 0 ? Math.min(limit, nodes.length) : nodes.length;
    for (let i = 0; i < n; i++) {
      const el = nodes[i];
      const r = el.getBoundingClientRect();
      const attrs = {};
      for (const a of el.attributes) attrs[a.name] = a.value;
      out.push({
        selector: window.__amq_path(el),
        tag: el.tagName.toLowerCase(),
        text: (el.textContent || '').slice(0, 500),
        attrs,
        rect: { x: r.x, y: r.y, width: r.width, height: r.height },
        focused: el === document.activeElement,
      });
    }
    return { matches: out };
  };
})();
