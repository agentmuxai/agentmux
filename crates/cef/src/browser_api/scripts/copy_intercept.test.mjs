// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// copy_intercept.js, the text capture_copy.rs runs in a page, against stand-ins for what a browser has: a
// clipboard that records what really reached it, a document, a data transfer and a copy event.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "copy_intercept.js"), "utf8");
const copyIntercept = new Function(`return (${src});`)();

const SECRET = "xK9fQ2mLp7ZaB3cD4eF5gH6";

/// A page: `real` is what reached the system clipboard.
function page() {
  const real = [];
  class Clipboard {
    writeText(text) {
      real.push(["writeText", text]);
      return Promise.resolve();
    }
    write(items) {
      real.push(["write", items]);
      return Promise.resolve();
    }
  }
  class Document {
    constructor() {
      this.handlers = [];
      this.activeElement = null;
      this.body = this.element();
    }
    element(extra = {}) {
      return { dispatchEvent: (e) => this.handlers.forEach((h) => h(e)), ...extra };
    }
    execCommand(command) {
      real.push(["execCommand", command]);
      return true;
    }
  }
  class DataTransfer {
    constructor() {
      this.data = {};
    }
    setData(type, value) {
      this.data[type] = value;
    }
    getData(type) {
      return this.data[type] || "";
    }
  }
  class ClipboardEvent {
    constructor(type, init) {
      this.type = type;
      this.clipboardData = init.clipboardData;
    }
  }
  const win = { Clipboard, Document, DataTransfer, ClipboardEvent, getSelection: () => "" };
  const document = new Document();
  const clipboard = new Clipboard();
  return { win, real, document, clipboard };
}

const install = (w, nonce = "__n1") => copyIntercept(w, nonce, "install");
const read = (w, nonce = "__n1") => copyIntercept(w, nonce, "read");
const restore = (w, nonce = "__n1") => copyIntercept(w, nonce, "restore");

describe("copyIntercept", () => {
  it("takes what a page writes with the async clipboard, and the system clipboard never gets it", async () => {
    const { win, real, clipboard } = page();
    install(win);
    await clipboard.writeText(SECRET);
    expect(read(win)).toEqual({ value: SECRET, how: "writeText" });
    expect(real).toEqual([]);
  });

  it("takes the text/plain part of a clipboard item", async () => {
    const { win, real, clipboard } = page();
    install(win);
    const item = { types: ["text/html", "text/plain"], getType: async (t) => ({ text: async () => (t === "text/plain" ? SECRET : "<b>x</b>") }) };
    await clipboard.write([item]);
    await new Promise((r) => setTimeout(r, 0));
    expect(read(win)).toEqual({ value: SECRET, how: "write" });
    expect(real).toEqual([]);
  });

  it("lets the page's copy handler say what is copied, and falls back to the selected text of the focused field", () => {
    const { win, real, document } = page();
    install(win);
    document.handlers.push((e) => e.clipboardData.setData("text/plain", SECRET));
    expect(document.execCommand("copy")).toBe(true);
    expect(read(win)).toEqual({ value: SECRET, how: "execCommand" });
    expect(real).toEqual([]);

    const second = page();
    install(second.win, "__n2");
    second.document.activeElement = second.document.element({ value: `key ${SECRET} end`, selectionStart: 4, selectionEnd: 4 + SECRET.length });
    second.document.execCommand("copy");
    expect(read(second.win, "__n2")).toEqual({ value: SECRET, how: "execCommand" });
    expect(second.real).toEqual([]);
  });

  it("takes the page's selection when no field is focused", () => {
    const { win, document } = page();
    win.getSelection = () => ({ toString: () => SECRET });
    install(win);
    document.execCommand("copy");
    expect(read(win).value).toBe(SECRET);
  });

  it("lets every other command through and keeps only the first thing copied", async () => {
    const { win, real, document, clipboard } = page();
    install(win);
    document.execCommand("bold");
    expect(real).toEqual([["execCommand", "bold"]]);
    expect(read(win)).toBeNull();
    await clipboard.writeText(SECRET);
    await clipboard.writeText("something else");
    expect(read(win).value).toBe(SECRET);
    await clipboard.writeText("   ");
    expect(read(win).value).toBe(SECRET);
  });

  it("restores the page's own functions and leaves nothing behind", async () => {
    const { win, real, clipboard } = page();
    const before = win.Clipboard.prototype.writeText;
    install(win);
    expect(win.Clipboard.prototype.writeText).not.toBe(before);
    expect(Object.keys(win)).not.toContain("__n1");
    restore(win);
    expect(win.Clipboard.prototype.writeText).toBe(before);
    expect("__n1" in win).toBe(false);
    await clipboard.writeText("later");
    expect(real).toEqual([["writeText", "later"]]);
  });

  it("is installed once", () => {
    const { win } = page();
    install(win);
    const first = win.Clipboard.prototype.writeText;
    install(win);
    expect(win.Clipboard.prototype.writeText).toBe(first);
  });
});
