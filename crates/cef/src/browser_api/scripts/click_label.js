// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The label a committing click is shown and judged by. act.rs splices this
// file into COMMIT_INFO as the value of `clickLabel`, so it must stay one
// function expression; click_label.test.mjs runs the same text.
//
// The name is the element's aria-label, then the text of the elements its
// aria-labelledby names, then the first non-empty line of its visible text
// (or a button input's value). A menu item's visible text often holds a
// description under its title ("Delete revision" over "This edit's content
// will no longer be visible"): the lines after the first are that
// description, shown smaller in the approval and never judged. Whether the
// click commits (a verb like Send or Delete) is read from the name only, and
// so is srv's payment check, which sees only the `Click "name"` line.
function clickLabel(ariaLabel, labelledBy, text) {
  const clean = (s) => String(s || "").replace(/\s+/g, " ").trim();
  const lines = String(text || "").split("\n").map(clean).filter(Boolean);
  const named = clean(ariaLabel) || clean(labelledBy);
  const name = (named || lines[0] || "").slice(0, 120);
  const description = (named ? "" : lines.slice(1).join(" ")).slice(0, 200);
  const verb = /\b(submit|send|pay|buy|purchase|order|checkout|check out|delete|remove|publish|post|confirm|sign up|sign|agree|accept|transfer|unsubscribe)\b/i.test(name);
  return { name, description, verb };
}
