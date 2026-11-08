function (doc, key) {
  // What Enter or Space would do on the element that has focus, named as a
  // click on a control (spec §5): the host makes that click through act's
  // checks and never presses the key. An effect it cannot name is refused.
  //   { effect: "click", el, focused, why }  click `el`
  //   { effect: "type", focused, why }       Space in a text field: a space, typed (by the host, in the same call)
  //   { effect: "refuse", why }
  // Properties are read through the prototypes' getters, which the page cannot
  // shadow on an element (a control named "type" or "form").
  const refuse = (why) => ({ effect: "refuse", why });
  try {
    let el = doc.activeElement;
    for (let i = 0; el && i < 16 && el.shadowRoot && el.shadowRoot.activeElement; i++) el = el.shadowRoot.activeElement;
    if (!el || el === doc.body || el === doc.documentElement) return refuse("nothing on the page has focus");
    const win = el.ownerDocument.defaultView;
    const name = el.localName;
    if (name === "iframe" || name === "frame") return refuse("the focus is inside a frame");
    if (el.isContentEditable) return refuse("it is an editable area, where the page decides what the key does");
    const get = (proto, prop, x) => Object.getOwnPropertyDescriptor(proto, prop).get.call(x);
    const typeOf = (x) => {
      const proto = x.localName === "button" ? win.HTMLButtonElement.prototype : x.localName === "input" ? win.HTMLInputElement.prototype : null;
      return proto ? String(get(proto, "type", x)).toLowerCase() : "";
    };
    const type = typeOf(el);
    if (el.matches(":disabled")) return refuse("it is disabled");
    const button = name === "button" || (name === "input" && ["submit", "button", "reset", "image"].includes(type));
    const choice = name === "input" && (type === "checkbox" || type === "radio");
    const textual = name === "textarea" || (name === "input" && ["text", "search", "url", "tel", "email", "password", "number", "date", "datetime-local", "month", "time", "week"].includes(type));
    const summary = name === "summary" && el.parentElement && el.parentElement.localName === "details";
    const click = (target, why) => ({ effect: "click", el: target, focused: el, why });
    if (key === "Space") {
      if (button || choice || summary) return click(el, "Space on " + (choice ? "a " + type : button ? "a button" : "a summary") + " presses it");
      if (textual) return el.readOnly ? refuse("it is read-only") : { effect: "type", focused: el, why: "Space in a text field types a space" };
      return refuse("Space on this element scrolls the page or is the page's own, and is not a click");
    }
    // Enter.
    if (button || summary) return click(el, "Enter on " + (summary ? "a summary" : "a button") + " presses it");
    if (name === "a" && el.hasAttribute("href")) return click(el, "Enter on a link follows it");
    if (textual && name === "textarea") return refuse("Enter in a text area adds a line, or is the page's own to send");
    if (textual || choice) {
      // Implicit submission: the form's default button, the first submit button in tree order.
      const form = get(win.HTMLInputElement.prototype, "form", el);
      if (!form) return refuse("it is not in a form, so Enter has no submit button to press");
      const submitter = (x) => (x.localName === "button" && typeOf(x) === "submit") || (x.localName === "input" && ["submit", "image"].includes(typeOf(x)));
      const formOf = (x) => get(x.localName === "button" ? win.HTMLButtonElement.prototype : win.HTMLInputElement.prototype, "form", x);
      // The form's own submit buttons in tree order: not form.elements, which leaves image buttons out.
      const first = Array.from(el.ownerDocument.querySelectorAll("button, input")).find((x) => submitter(x) && formOf(x) === form);
      if (!first) return refuse("its form has no submit button");
      if (first.matches(":disabled")) return refuse("its form's submit button is disabled");
      return click(first, "Enter in a field of a form presses the form's default button");
    }
    return refuse("Enter on this element has an effect the page defines");
  } catch (e) {
    return refuse("the page's markup could not be read (" + e + ")");
  }
}
