// @vitest-environment jsdom
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// press_effect.js, the rule press.rs names Enter's and Space's click by (spec §5),
// run as the same text the host splices in, on the controls of a jsdom page.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeEach, describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "press_effect.js"), "utf8");
const effect = new Function(`return (${src});`)();

/** Put `html` on the page, focus `#f`, and say what `key` would do there. */
function press(html, key) {
    document.body.innerHTML = html;
    document.getElementById("f")?.focus();
    return effect(document, key);
}

beforeEach(() => {
    document.body.innerHTML = "";
});

describe("Enter", () => {
    it("in a field of a form presses the form's default button", () => {
        const got = press(`<form><input id="f" name="q"><button id="b">Search</button></form>`, "Enter");
        expect(got.effect).toBe("click");
        expect(got.el.id).toBe("b");
        expect(got.focused.id).toBe("f");
    });

    it("takes the first submit button in tree order, one outside the form by its form attribute included", () => {
        const got = press(`<form id="x"><input id="f"><button type="button">Clear</button><button id="first">Go</button><button>Other</button></form>`, "Enter");
        expect(got.el.id).toBe("first");
        const outside = press(`<button id="out" form="x">Outside</button><form id="x"><input id="f"><button id="in">Go</button></form>`, "Enter");
        expect(outside.el.id).toBe("out");
        // A <button type="bogus"> is a submit button, as the browser reads it.
        const bogus = press(`<form><input id="f"><button id="b" type="bogus">Go</button></form>`, "Enter");
        expect(bogus.el.id).toBe("b");
    });

    it("counts an image button as a submit button, which form.elements leaves out", () => {
        const got = press(`<form><input id="f"><input type="image" id="img" alt="Go"><button id="later">Other</button></form>`, "Enter");
        expect(got.effect).toBe("click");
        expect(got.el.id).toBe("img");
        // Its own form's buttons only: a button of another form earlier on the page is not the default.
        const other = press(`<form><button id="elsewhere">Elsewhere</button></form><form><input id="f"><button id="mine">Mine</button></form>`, "Enter");
        expect(other.el.id).toBe("mine");
    });

    it("finds the default button of a form inside a shadow root", () => {
        document.body.innerHTML = `<div id="host"></div>`;
        const shadow = document.getElementById("host").attachShadow({ mode: "open" });
        shadow.innerHTML = `<form><input id="f"><button id="inside">Go</button></form>`;
        shadow.getElementById("f").focus();
        const got = effect(document, "Enter");
        expect(got.effect).toBe("click");
        expect(got.el.id).toBe("inside");
    });

    it("is refused when the form has no submit button, or its button is disabled", () => {
        expect(press(`<form><input id="f"></form>`, "Enter").effect).toBe("refuse");
        expect(press(`<form><input id="f"><button disabled>Go</button></form>`, "Enter").effect).toBe("refuse");
    });

    it("is refused in a field that is in no form", () => {
        const got = press(`<input id="f"><button>Search</button>`, "Enter");
        expect(got.effect).toBe("refuse");
        expect(got.why).toMatch(/not in a form/);
    });

    it("in a password field is the form's button too: the field is not written to", () => {
        const got = press(`<form><input id="f" type="password"><button id="b">Sign in</button></form>`, "Enter");
        expect(got.effect).toBe("click");
        expect(got.el.id).toBe("b");
    });

    it("presses the focused button, link and summary", () => {
        expect(press(`<button id="f" type="button">Plain</button>`, "Enter").el.id).toBe("f");
        expect(press(`<a id="f" href="#x">A link</a>`, "Enter").el.id).toBe("f");
        expect(press(`<details><summary id="f">More</summary>text</details>`, "Enter").el.id).toBe("f");
    });

    it("is refused in a text area, an editable area, a select, a link without href and a custom widget", () => {
        expect(press(`<form><textarea id="f"></textarea><button>Go</button></form>`, "Enter").effect).toBe("refuse");
        expect(press(`<select id="f"><option>a</option></select>`, "Enter").effect).toBe("refuse");
        expect(press(`<a id="f">no target</a>`, "Enter").effect).toBe("refuse");
        expect(press(`<div id="f" role="button" tabindex="0">Widget</div>`, "Enter").effect).toBe("refuse");
    });

    it("is refused when nothing has focus, or the focus is in a frame", () => {
        expect(press(`<button>Go</button>`, "Enter").effect).toBe("refuse");
        expect(press(`<iframe id="f"></iframe>`, "Enter").effect).toBe("refuse");
    });

    it("cannot be fooled by a control named like a form property", () => {
        // <input name="type"> and <input name="form"> shadow the form's own properties; the prototype's getters don't.
        const got = press(`<form><input id="f" name="elements"><input name="type"><input name="action"><button id="b">Go</button></form>`, "Enter");
        expect(got.effect).toBe("click");
        expect(got.el.id).toBe("b");
    });
});

describe("Space", () => {
    it("presses a button and toggles a checkbox or radio", () => {
        expect(press(`<button id="f" type="button">Plain</button>`, "Space").effect).toBe("click");
        const box = press(`<input id="f" type="checkbox">`, "Space");
        expect(box.effect).toBe("click");
        expect(box.el.id).toBe("f");
        expect(press(`<input id="f" type="radio">`, "Space").effect).toBe("click");
    });

    it("in a text field types a space, and a read-only field is refused", () => {
        expect(press(`<input id="f">`, "Space").effect).toBe("type");
        expect(press(`<textarea id="f"></textarea>`, "Space").effect).toBe("type");
        expect(press(`<input id="f" readonly>`, "Space").effect).toBe("refuse");
    });

    it("is refused on a link and a custom widget", () => {
        expect(press(`<a id="f" href="#x">A link</a>`, "Space").effect).toBe("refuse");
        expect(press(`<div id="f" role="button" tabindex="0">Widget</div>`, "Space").effect).toBe("refuse");
    });
});
