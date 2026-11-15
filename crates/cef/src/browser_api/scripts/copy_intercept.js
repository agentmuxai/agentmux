// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Takes what a page's own Copy button copies, without the system clipboard (#47629). capture_copy.rs
// runs this in the page's own world, as `(<this file>)(window, nonce, mode)`; copy_intercept.test.mjs runs
// the same text against stand-ins for the browser's objects.
//
// "install" puts a stand-in over the three ways a page copies, so the text goes to this script and the
// clipboard is never written:
// - navigator.clipboard.writeText(text) and navigator.clipboard.write([item]) (the text/plain part);
// - document.execCommand("copy"), which first lets the page's own `copy` handler say what to copy (a
//   handler that calls event.clipboardData.setData), else takes the selected text of the focused field or
//   of the page.
// Any other execCommand goes on to the browser. The first text copied is kept; the rest are let through
// quietly so the page believes its copy worked.
// "read" answers {value, how} once something was copied, else null. "restore" puts everything back.
//
// It is the page's world, so the page could look at what is kept here: the page already shows the
// value. A page that kept its own reference to the browser's clipboard function before this ran writes the
// clipboard anyway, which this can't see; capture_copy.rs says so when nothing was taken.
function copyIntercept(win, nonce, mode) {
  if (mode === "read") {
    const b = win[nonce];
    return b && b.value !== null ? { value: b.value, how: b.how } : null;
  }
  if (mode === "restore") {
    const b = win[nonce];
    if (b) {
      for (const [obj, name, desc] of b.patches) Object.defineProperty(obj, name, desc);
      delete win[nonce];
    }
    return true;
  }
  if (win[nonce]) return true;
  const box = { value: null, how: "", patches: [] };
  const take = (text, how) => {
    if (box.value === null && typeof text === "string" && text.trim() !== "") {
      box.value = text;
      box.how = how;
    }
  };
  const patch = (obj, name, make) => {
    if (!obj) return;
    const desc = Object.getOwnPropertyDescriptor(obj, name);
    if (!desc || typeof desc.value !== "function") return;
    box.patches.push([obj, name, desc]);
    Object.defineProperty(obj, name, Object.assign({}, desc, { value: make(desc.value) }));
  };
  const clip = win.Clipboard && win.Clipboard.prototype;
  patch(clip, "writeText", () => function (text) {
    take(String(text), "writeText");
    return Promise.resolve();
  });
  patch(clip, "write", () => function (items) {
    for (const item of Array.from(items || [])) {
      try {
        if (item.types && item.types.includes("text/plain")) {
          item.getType("text/plain").then((blob) => blob.text()).then((text) => take(text, "write"));
        }
      } catch (e) {
        // An item the page built wrongly copies nothing, as it would in the browser.
      }
    }
    return Promise.resolve();
  });
  patch(win.Document && win.Document.prototype, "execCommand", (original) => function (command) {
    const c = String(command).toLowerCase();
    if (c !== "copy" && c !== "cut") return original.apply(this, arguments);
    const doc = this;
    let text = "";
    try {
      const data = new win.DataTransfer();
      const event = new win.ClipboardEvent("copy", { clipboardData: data, bubbles: true, cancelable: true });
      (doc.activeElement || doc.body).dispatchEvent(event);
      text = data.getData("text/plain");
    } catch (e) {
      text = "";
    }
    if (!text) {
      const a = doc.activeElement;
      if (a && typeof a.value === "string" && a.selectionEnd > a.selectionStart) {
        text = a.value.substring(a.selectionStart, a.selectionEnd);
      } else {
        text = String(win.getSelection ? win.getSelection() : "");
      }
    }
    take(text, "execCommand");
    return true;
  });
  Object.defineProperty(win, nonce, { value: box, enumerable: false, configurable: true });
  return true;
}
