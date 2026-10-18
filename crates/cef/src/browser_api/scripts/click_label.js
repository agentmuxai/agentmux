// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How a click is named, and whether it commits (submits, sends, pays,
// deletes) so the person is asked first. act.rs splices this file into
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
// `el` is what COMMIT_INFO read from the element: `submits` (it submits a
// form), `link` (it is, or is inside, a link outside a form), and for a link
// `download` (it has a download attribute) and `fileHref` (its href is a
// file: URL).
//
// - Anything that submits a form commits.
// - A button, or anything else that isn't a link, commits when a committing
//   word is anywhere in its name or description: the split is for display
//   only, so a later line can only ever make a click ask, never stop it
//   asking ("$20.00" over "Pay now", "Archive" over "This deletes it").
// - A link commits when its name starts with a committing word, or is short
//   (at most LINK_WORDS words) and holds one. A link's name is often a
//   sentence ("This acknowledgement does not confirm the accuracy ... Click
//   here to download a PDF"), where a word like "confirm" means nothing. A
//   download link or a file: link never commits: it fetches, it sends nothing.
//
// `why` says what made it commit, for the approval banner.
function clickLabel(ariaLabel, labelledBy, text, el) {
  const LINK_WORDS = 6;
  const VERBS = "submit|send|pay|buy|purchase|order|checkout|check out|delete|remove|publish|post|confirm|sign up|sign|agree|accept|transfer|unsubscribe";
  const clean = (s) => String(s || "").replace(/\s+/g, " ").trim();
  const lines = String(text || "").split("\n").map(clean).filter(Boolean);
  const named = clean(ariaLabel) || clean(labelledBy);
  const fullName = named || lines[0] || "";
  const rest = named ? "" : lines.slice(1).join(" ");
  const name = fullName.slice(0, 120);
  const description = rest.slice(0, 200);
  const e = el || {};
  const found = (s, re) => {
    const m = re.exec(s);
    return m ? m[1] : "";
  };
  const anywhere = new RegExp("\\b(" + VERBS + ")\\b", "i");
  let committing = false;
  let why = "";
  if (e.submits) {
    committing = true;
    why = "it submits a form";
  } else if (e.link) {
    if (!e.download && !e.fileHref) {
      const short = fullName.split(" ").filter(Boolean).length <= LINK_WORDS;
      const word = found(fullName, new RegExp("^(" + VERBS + ")\\b", "i")) || (short ? found(fullName, anywhere) : "");
      if (word) {
        committing = true;
        why = "its name says \"" + word + "\"";
      }
    }
  } else {
    // A description is a sentence ("This deletes it permanently"): there a
    // word counts with its -s, -es, -d or -ed. A name is a command, and a tab
    // named "Orders" or "Posts" doesn't commit.
    const inName = found(fullName, anywhere);
    const inRest = inName ? "" : found(rest, new RegExp("\\b((?:" + VERBS + ")(?:s|es|d|ed)?)\\b", "i"));
    if (inName || inRest) {
      committing = true;
      why = inName ? "its name says \"" + inName + "\"" : "its text says \"" + inRest + "\"";
    }
  }
  return { name, description, committing, why };
}
