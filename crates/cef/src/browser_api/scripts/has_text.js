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
((t) => {
  const boxed = (el) => !!el && el.getClientRects().length > 0;
  return !!document.body && (boxed(document.documentElement) || boxed(document.body)) && document.body.innerText.includes(t);
})
