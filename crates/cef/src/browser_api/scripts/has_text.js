// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Does this frame show `t`? wait.rs runs this in an isolated world, once per
// frame, with the text as its argument, so it must stay one function
// expression; has_text.test.mjs runs the same text.
//
// A frame nobody can see (its `<iframe>` is `display: none`, as the Azure
// portal leaves the blade it just left) has no layout at all, and `innerText`
// then falls back to the raw text. So the text only counts while the page has a
// box: the root's or the body's (a `display: contents` body has none of its
// own, and its children still render).
//
// A secret the page shows once doesn't count (spec §15.1): where a page shows
// one (code, pre, kbd, samp, a Copy button's container), words with its shape
// are left out, and an element holding a captured Vault value is left out
// whole, so waiting for a piece of one can't read it a piece at a time. The
// shape is a superset of agentmux_common::shown_once's.
((t) => {
  const boxed = (el) => !!el && el.getClientRects().length > 0;
  if (!document.body || !(boxed(document.documentElement) || boxed(document.body))) return false;
  let text = document.body.innerText;
  if (!text.includes(t)) return false;
  const could = (w) => w.length >= 20 && w.length <= 512 && !/^[0-9a-fA-F]+$/.test(w)
    && !/^\{?[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}\}?$/.test(w) && !/:\/\/|^www\.|^mailto:/i.test(w)
    && [/[a-z]/, /[A-Z]/, /[0-9]/, /[^A-Za-z0-9]/].filter((c) => c.test(w)).length >= 2;
  const zones = [...document.querySelectorAll("[data-agentmux-vault], code, pre, kbd, samp")];
  for (const b of document.querySelectorAll("button, a, [role=button]")) {
    const name = ((b.textContent || "") + " " + (b.getAttribute("aria-label") || "") + " " + (b.getAttribute("title") || "")).toLowerCase();
    if (!name.includes("copy")) continue;
    let c = b.parentElement;
    for (let i = 0; c && c.children.length <= 1 && i < 4; i++) c = c.parentElement;
    if (c) zones.push(c);
  }
  for (const z of zones) {
    // An element holding a captured value is left out whole: its value may not
    // have the shape.
    const whole = z.hasAttribute("data-agentmux-vault") ? String(z.innerText || z.textContent || "").trim() : "";
    if (whole) text = text.split(whole).join(" ");
    for (const w of String(z.textContent || "").split(/\s+/)) {
      if (could(w)) text = text.split(w).join(" ");
    }
  }
  return text.includes(t);
})
