// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The label a committing click is shown by. act.rs splices this file into
// COMMIT_INFO as the value of `clickLabel`, so it must stay one function
// expression; click_label.test.mjs runs the same text.
//
// The name is the element's aria-label, then the text of the elements its
// aria-labelledby names, then the first non-empty line of its visible text
// (or a button input's value). A menu item's visible text often holds a
// description under its title ("Delete revision" over "This edit's content
// will no longer be visible"): the lines after the first are that
// description, which the approval shows smaller, under `Click "name"`.
//
// The split is for display only. Whether the click commits (a verb like Send
// or Delete) is read from the name and the description together, as is srv's
// payment check, so a later line can only ever make a click ask, never stop
// it asking ("$20.00" over "Pay now").
function clickLabel(ariaLabel, labelledBy, text) {
  const clean = (s) => String(s || "").replace(/\s+/g, " ").trim();
  const lines = String(text || "").split("\n").map(clean).filter(Boolean);
  const named = clean(ariaLabel) || clean(labelledBy);
  const rest = named ? "" : lines.slice(1).join(" ");
  const name = (named || lines[0] || "").slice(0, 120);
  const description = rest.slice(0, 200);
  const verb = /\b(submit|send|pay|buy|purchase|order|checkout|check out|delete|remove|publish|post|confirm|sign up|sign|agree|accept|transfer|unsubscribe)\b/i.test(
    `${named || lines[0] || ""} ${rest}`
  );
  return { name, description, verb };
}
