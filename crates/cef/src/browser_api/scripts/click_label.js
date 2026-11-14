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
// - "Pay" in a brand or a plan's name is not a command: "Kakao Pay", "Cash App Pay", "Apple Pay" and
//   "Pay-as-you-go" are rows, tabs and links that open a panel, and a page of payment methods has dozens.
//   The brand is one to three capitalised words in front of "Pay", none of them a verb or "and", "to",
//   "with": "Pay", "Pay now", "Pay $5.46", "Confirm and Pay" and "Pay with Apple Pay" still commit.
//
// `why` says what made it commit, for the approval banner.
//
// `axName` is the accessible name the browser computed for the element
// (`Accessibility.getPartialAXTree`), the one a snapshot or a find shows and
// the agent chose the element by. The element's own aria-label and text miss a
// name that comes from a title, an image's alt, a child's aria-label or CSS
// content, so the same rule runs on the accessible name too, and the click
// commits if either says so. `undefined`: not looked up. `null`: the lookup
// failed, and an element with no name of its own then commits (`unreadable`):
// a click that can't be named is asked about, never passed as harmless.
function clickLabel(ariaLabel, labelledBy, text, el, axName) {
  const LINK_WORDS = 6;
  const VERBS = "submit|send|pay|buy|purchase|order|checkout|check out|delete|remove|publish|post|confirm|sign up|sign|agree|accept|transfer|unsubscribe";
  const clean = (s) => String(s || "").replace(/\s+/g, " ").trim();
  // A plan's name ("Pay-as-you-go", "Pay-per-use") is not a word that commits: it is taken out of a name or a
  // description before they are judged. A brand ("Kakao Pay") is not one either, but only when it is the whole
  // name: one to three capitalised words and a last "Pay", none of the words a verb or a joiner (in any case:
  // "Confirm And Pay", "CLICK TO PAY"). Any doubt leaves the name as it is, and it asks. Keep in step with
  // is_brand_pay in browser_bypass.rs, whose list holds every word of VERBS.
  const NOT_BRAND = new Set("pay buy confirm complete place make click tap press finish submit send and to with now then or the a your proceed continue review order checkout check out purchase subscribe unsubscribe donate go use sign up agree accept transfer delete remove publish post securely express".split(" "));
  const BRAND_PAY = /^(?:[A-Z][\w.&'-]*\s+){1,3}P(?:ay|AY)$/;
  const planOnly = (s) => String(s || "").replace(/\bpay-(?:as|per)-[\w-]+/gi, " ");
  const plain = (s) => {
    const name = clean(planOnly(s)).replace(/[.!]+$/, "");
    return BRAND_PAY.test(name) && !name.split(" ").slice(0, -1).some((w) => NOT_BRAND.has(w.toLowerCase())) ? "" : planOnly(s);
  };
  const lines = String(text || "").split("\n").map(clean).filter(Boolean);
  const named = clean(ariaLabel) || clean(labelledBy);
  const fullName = named || lines[0] || "";
  const rest = named ? "" : lines.slice(1).join(" ");
  const name = fullName.slice(0, 120);
  // Long enough for srv's payment check to see a word far down; the banner
  // shows less.
  const description = rest.slice(0, 1000);
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
      const word = found(clean(plain(fullName)), new RegExp("^(" + VERBS + ")\\b", "i")) || (short ? found(plain(fullName), anywhere) : "");
      if (word) {
        committing = true;
        why = "its name says \"" + word + "\"";
      }
    }
  } else {
    // A description is a sentence ("This deletes it permanently"): there a
    // word counts with its -s, -es, -d or -ed. A name is a command, and a tab
    // named "Orders" or "Posts" doesn't commit.
    const inName = found(plain(fullName), anywhere);
    const inRest = inName ? "" : found(planOnly(rest), new RegExp("\\b((?:" + VERBS + ")(?:s|es|d|ed)?)\\b", "i"));
    if (inName || inRest) {
      committing = true;
      why = inName ? "its name says \"" + inName + "\"" : "its text says \"" + inRest + "\"";
    }
  }
  // A link with text of its own is named by it: its accessible name repeats that text with the description
  // joined on ("Billing history Pay an invoice"), which would make a sentence's word commit.
  if (typeof axName === "string" && axName.trim() && !(e.link && name)) {
    const byAx = clickLabel(axName, "", "", e);
    return {
      name: name || byAx.name,
      description,
      committing: committing || byAx.committing,
      why: committing ? why : byAx.why,
    };
  }
  if (axName === null && !name) {
    return { name, description, committing: true, why: "its name could not be read", unreadable: true };
  }
  return { name, description, committing, why };
}
