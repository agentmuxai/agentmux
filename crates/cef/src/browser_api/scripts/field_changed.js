// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Whether a form control holds something other than what its page loaded it with. act.rs splices this file
// into COMMIT_INFO as the value of `fieldChanged`, so it must stay one function expression;
// field_changed.test.mjs runs the same text. The approval prompt for a click that submits a form lists every
// field the form posts, and marks the ones the agent filled apart from the values the page already had (a
// site's placeholders, a saved address): a field whose value is its default is "already on the page".
//
// `ty` is the control's type. A checkbox or radio is listed only when checked, so it counts as changed when it
// is checked and was not by default (or the reverse); a file input when it holds a file.
function fieldChanged(el, ty) {
  if (ty === "checkbox" || ty === "radio") return el.checked !== el.defaultChecked;
  if (ty === "file") return !!(el.files && el.files.length > 0);
  if (el.tagName === "SELECT") return Array.from(el.options).some((o) => o.selected !== o.defaultSelected);
  return el.value !== el.defaultValue;
}
