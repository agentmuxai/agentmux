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

  // Build the `[data-blockid="..."]` selector for a pane's own DOM
  // subtree wrapper (set by frontend/app/block/blockframe.tsx on every
  // pane's root element). CSS.escape avoids selector-injection from a
  // block_id containing quotes/special chars — this string is built here,
  // in JS, rather than concatenated in Rust, specifically so it's safe by
  // construction regardless of what block_id contains.
  window.__amq_block_selector = (blockId) =>
    blockId ? `[data-blockid="${CSS.escape(blockId)}"]` : null;

  // Ownership check for click/query/focus (NOT screenshot — see
  // __amq_rect_of below for why that one stays strictly pane-only).
  // `blockId` falsy → no ownership concept applies (the browser-pane
  // case, where the CDP target already IS that block's own isolated
  // page). Otherwise: an element is allowed if it's either (a) not
  // inside ANY pane's [data-blockid] wrapper — shared app chrome
  // (status bar, hamburger menu, window controls), not owned by any one
  // agent, so reaching it isn't the kind of privacy leak reaching
  // another agent's pane would be — or (b) inside blockId's OWN pane.
  // Never allowed: inside a DIFFERENT block's [data-blockid] wrapper.
  // See §6 of SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md.
  window.__amq_allowed_for = (el, blockId) => {
    if (!blockId) return true;
    const owner = el.closest('[data-blockid]');
    if (!owner) return true;
    return owner.getAttribute('data-blockid') === blockId;
  };

  // Return viewport-space centroid of the first ALLOWED selector match
  // (see __amq_allowed_for), or null if no allowed element matches. Used
  // by /agentmux/browser/click_element to convert a CSS selector into
  // Input.dispatchMouseEvent coords.
  window.__amq_centroid_of = (selector, blockId) => {
    let nodes;
    try { nodes = document.querySelectorAll(selector); } catch (e) { return null; }
    const el = Array.from(nodes).find((e) => window.__amq_allowed_for(e, blockId));
    if (!el) return null;
    // Scroll into view so the centroid is actually clickable. Chromium
    // silently drops dispatchMouseEvent events that land outside the
    // visible viewport; without the scroll, an off-screen selector
    // would look like a successful call that does nothing.
    el.scrollIntoView({ block: 'center', inline: 'center' });
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  };

  // getBoundingClientRect() of blockId's own pane wrapper — used to build
  // a CDP Page.captureScreenshot `clip` so a screenshot only covers one
  // pane's subtree, not the whole shared window. Deliberately does NOT
  // get the same "shared chrome is fine" exception __amq_allowed_for
  // gives click/query: a screenshot's clip is a single rectangle that
  // can't punch a hole for another agent's pane sitting geometrically
  // between the caller's pane and a chrome element, so unlike an
  // element-level filter it can't safely be widened — it stays strictly
  // pane-only, always. Returns null if the pane isn't found (caller
  // falls back to full-viewport capture).
  window.__amq_rect_of = (blockId) => {
    const sel = window.__amq_block_selector(blockId);
    if (!sel) return null;
    let el;
    try { el = document.querySelector(sel); } catch (e) { return null; }
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  };

  // Focus the first ALLOWED selector match (see __amq_allowed_for).
  // Returns true/false, never throws — used by
  // /agentmux/browser/focus_element and (to focus before typing)
  // /agentmux/browser/dispatch_key.
  window.__amq_focus = (selector, blockId) => {
    let nodes;
    try { nodes = document.querySelectorAll(selector); } catch (e) { return false; }
    const el = Array.from(nodes).find((e) => window.__amq_allowed_for(e, blockId));
    if (!el) return false;
    el.focus();
    return true;
  };

  window.__amq_query = (selector, limit, blockId) => {
    let nodes;
    try {
      nodes = document.querySelectorAll(selector);
    } catch (e) {
      return { error: String(e) };
    }
    const allowed = Array.from(nodes).filter((e) => window.__amq_allowed_for(e, blockId));
    const out = [];
    const n = limit > 0 ? Math.min(limit, allowed.length) : allowed.length;
    for (let i = 0; i < n; i++) {
      const el = allowed[i];
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
