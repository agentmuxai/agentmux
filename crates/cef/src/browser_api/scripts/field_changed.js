// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Whether a form control holds something other than what its page loaded it with, and whether the agent
// wrote to it. act.rs splices this file into COMMIT_INFO as the value of `fieldChanged`, so it must stay one
// function expression; field_changed.test.mjs runs the same text. The approval prompt for a click that
// submits a form lists every field the form posts, and marks the ones the agent filled apart from the values
// the page already had (a site's placeholders, a saved address): a field is "already on the page" only when
// its value is its default AND the agent never wrote to it. A page that keeps its fields in sync with its own
// state (a controlled input) moves the default along with what was typed, so the default alone proves
// nothing; every write the host makes sets `data-agentmux-filled` on the field.
//
// `ty` is the control's type. A checkbox or radio is listed only when checked, so it counts as changed when it
// is checked and was not by default (or the reverse); a file input when it holds a file. A single-choice
// select with no option marked by the page shows its first enabled option: that is its default.
function fieldChanged(el, ty) {
  if (typeof el.hasAttribute === "function" && (el.hasAttribute("data-agentmux-filled") || el.hasAttribute("data-agentmux-vault"))) return true;
  if (ty === "checkbox" || ty === "radio") return el.checked !== el.defaultChecked;
  if (ty === "file") return !!(el.files && el.files.length > 0);
  if (el.tagName === "SELECT") {
    const options = Array.from(el.options);
    const first = el.multiple || options.some((o) => o.defaultSelected) ? null : options.find((o) => !o.disabled);
    return options.some((o) => o.selected !== (o.defaultSelected || o === first));
  }
  return el.value !== el.defaultValue;
}
