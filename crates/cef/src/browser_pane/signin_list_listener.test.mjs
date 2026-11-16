// @vitest-environment jsdom
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The list of saved sign-ins under a login field, page side
// (docs/specs/SPEC_VAULT_AUTOFILL_IN_BROWSER_PANE_2026_11_14.md §P2.3, §P2.4): `signin_notice.js` is
// installed on a stand-in window, sent the events the person's input makes, and what it reports and which
// keys it swallows is read back. A page's own events (`isTrusted` false) report nothing, and a password's
// value is never in a report.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const NOTICE = readFileSync(join(here, "signin_notice.js"), "utf8");

beforeAll(() => {
    Element.prototype.getClientRects = function () {
        return [{}];
    };
});
afterEach(() => {
    document.body.innerHTML = "";
});

// The listener on a stand-in window that keeps every handler (a type can have several) and what it reports.
function install() {
    const handlers = {};
    const reports = [];
    const extra = {
        addEventListener: (type, fn) => (handlers[type] ||= []).push(fn),
        agentmuxSignIn: (json) => reports.push(JSON.parse(json)),
    };
    const win = new Proxy(window, {
        get: (target, key) => (key in extra ? extra[key] : typeof target[key] === "function" ? target[key].bind(target) : target[key]),
        set: (target, key, value) => ((extra[key] = value), true),
    });
    new Function("window", NOTICE)(win);
    // One event through every handler of its type, in the order they were added, until one stops it.
    const fire = (type, init) => {
        let stopped = false;
        const e = { isTrusted: true, preventDefault: vi.fn(), stopImmediatePropagation: vi.fn(() => (stopped = true)), ...init };
        for (const fn of handlers[type] || []) {
            if (stopped) break;
            fn(e);
        }
        return e;
    };
    return { win, reports, fire, lists: () => reports.filter((r) => ["open", "key", "close"].includes(r.kind)) };
}

const field = (id) => document.getElementById(id);
const SIGNIN = `<form><input id=user type=text name=username autocomplete="username"><input id=pw type=password autocomplete="current-password"></form>`;

describe("the list of saved sign-ins, page side", () => {
    it("opens on the person's click in a password field, a user name field and a field for a new password", () => {
        document.body.innerHTML = SIGNIN + `<form><input id=n type=password autocomplete="new-password"></form>`;
        const l = install();
        l.fire("pointerdown", { target: field("pw") });
        l.fire("pointerdown", { target: field("user") });
        l.fire("pointerdown", { target: field("n") });
        expect(l.lists()).toEqual([
            { kind: "open", role: "password" },
            { kind: "open", role: "username", text: "" },
            { kind: "open", role: "newpassword" },
        ]);
    });

    it("sends the text typed in a user name field, clipped, and never any other field's text", () => {
        document.body.innerHTML = SIGNIN;
        field("user").value = "alice@example.com".padEnd(100, "x");
        field("pw").value = "hunter2-the-password";
        const l = install();
        l.fire("pointerdown", { target: field("user") });
        l.fire("pointerdown", { target: field("pw") });
        const [username, password] = l.lists();
        expect(username.text).toHaveLength(64);
        expect("text" in password && password.text !== undefined).toBe(false);
        expect(JSON.stringify(l.reports)).not.toContain("hunter2");
    });

    it("counts only the person's own events: a page's click, key or input reports nothing", () => {
        document.body.innerHTML = SIGNIN;
        const l = install();
        l.fire("pointerdown", { isTrusted: false, target: field("pw") });
        l.fire("keydown", { isTrusted: false, key: "ArrowDown", target: field("pw") });
        expect(l.lists()).toEqual([]);
        l.fire("pointerdown", { target: field("pw") });
        l.fire("input", { isTrusted: false, target: field("user") });
        expect(l.lists()).toEqual([{ kind: "open", role: "password" }]);
    });

    it("does not offer the list in a sign-up form's user name field or the new password of a change form", () => {
        document.body.innerHTML =
            `<form><input id=su type=text autocomplete="username"><input id=sp type=password autocomplete="new-password"></form>` +
            `<form><input id=cur type=password autocomplete="current-password"><input id=np type=password autocomplete="new-password"></form>`;
        const l = install();
        l.fire("pointerdown", { target: field("su") });
        l.fire("pointerdown", { target: field("np") });
        expect(l.lists()).toEqual([]);
        // The change form's current password is an ordinary password field.
        l.fire("pointerdown", { target: field("cur") });
        expect(l.lists().filter((r) => r.kind === "open").map((r) => r.role)).toEqual(["password"]);
    });

    it("does not offer the list in a user name field with no password field (the first step of a user-name-first sign-in)", () => {
        document.body.innerHTML = `<form><input id=first type=email name=email autocomplete="username"><button>Next</button></form>`;
        const l = install();
        l.fire("pointerdown", { target: field("first") });
        l.fire("keydown", { key: "ArrowDown", target: field("first") });
        expect(l.lists()).toEqual([]);
    });

    it("a press anywhere else closes it, once", () => {
        document.body.innerHTML = SIGNIN + "<p id=else>text</p>";
        const l = install();
        l.fire("pointerdown", { target: field("pw") });
        l.fire("pointerdown", { target: field("else") });
        l.fire("pointerdown", { target: field("else") });
        expect(l.lists()).toEqual([{ kind: "open", role: "password" }, { kind: "close" }]);
    });

    it("Up or Down in a field opens it and the page still gets the key", () => {
        document.body.innerHTML = SIGNIN;
        const l = install();
        const e = l.fire("keydown", { key: "ArrowDown", target: field("pw") });
        expect(l.lists()).toEqual([{ kind: "open", role: "password" }]);
        expect(e.preventDefault).not.toHaveBeenCalled();
        l.fire("keydown", { key: "a", target: field("pw") });
        expect(l.lists()).toHaveLength(1);
    });

    it("swallows Up, Down and Escape only while a list shows, and Enter only with a row selected", () => {
        document.body.innerHTML = SIGNIN;
        const l = install();
        l.fire("pointerdown", { target: field("pw") });
        l.win.__agentmuxList = { shown: true, cursor: false };
        for (const key of ["ArrowDown", "ArrowUp", "Escape"]) {
            const e = l.fire("keydown", { key, target: field("pw") });
            expect(e.preventDefault, key).toHaveBeenCalled();
            expect(e.stopImmediatePropagation, key).toHaveBeenCalled();
        }
        // Enter and Tab with nothing selected belong to the page.
        for (const key of ["Enter", "Tab"]) {
            const e = l.fire("keydown", { key, target: field("pw") });
            expect(e.preventDefault, key).not.toHaveBeenCalled();
        }
        expect(l.lists().filter((r) => r.kind === "key").map((r) => r.key)).toEqual(["ArrowDown", "ArrowUp", "Escape"]);
        l.win.__agentmuxList = { shown: true, cursor: true };
        const enter = l.fire("keydown", { key: "Enter", target: field("pw") });
        expect(enter.preventDefault).toHaveBeenCalled();
        // Tab picks the selected row and still moves focus.
        const tab = l.fire("keydown", { key: "Tab", target: field("pw") });
        expect(tab.preventDefault).not.toHaveBeenCalled();
        expect(l.lists().slice(-2).map((r) => r.key)).toEqual(["Enter", "Tab"]);
    });

    it("an Enter the list swallowed is never reported as a sign-in", () => {
        document.body.innerHTML = SIGNIN;
        field("user").value = "alice";
        field("pw").value = "hunter2";
        const l = install();
        l.fire("pointerdown", { target: field("pw") });
        l.win.__agentmuxList = { shown: true, cursor: true };
        l.fire("keydown", { key: "Enter", target: field("pw") });
        // The page's form is reported once at install; the swallowed Enter is only a key for the list.
        expect(l.reports.map((r) => r.kind).filter((k) => k !== "form")).toEqual(["open", "key"]);
    });

    it("follows what is typed in a user name field once the list was asked for", () => {
        document.body.innerHTML = SIGNIN;
        const l = install();
        l.fire("input", { target: field("user") });
        expect(l.lists()).toEqual([]);
        l.fire("pointerdown", { target: field("user") });
        field("user").value = "al";
        l.fire("input", { target: field("user") });
        field("pw").value = "x";
        l.fire("input", { target: field("pw") });
        expect(l.lists().map((r) => [r.kind, r.text])).toEqual([["open", ""], ["open", "al"]]);
    });

    it("closes when focus leaves the field, and a key in another field is the page's", () => {
        document.body.innerHTML = SIGNIN + "<input id=search type=text name=q>";
        const l = install();
        l.fire("pointerdown", { target: field("pw") });
        l.win.__agentmuxList = { shown: true, cursor: true };
        // Tab to the search box, then Down and Enter there: the page's keys, nothing swallowed.
        l.fire("focusin", { target: field("search") });
        expect(l.lists().map((r) => r.kind)).toEqual(["open", "close"]);
        for (const key of ["ArrowDown", "Enter"]) {
            const e = l.fire("keydown", { key, target: field("search") });
            expect(e.preventDefault, key).not.toHaveBeenCalled();
        }
        expect(l.lists().map((r) => r.kind)).toEqual(["open", "close"]);
    });

    it("a key in another field before the host heard of the focus change is the page's too", () => {
        document.body.innerHTML = SIGNIN + "<input id=search type=text name=q>";
        const l = install();
        l.fire("pointerdown", { target: field("pw") });
        l.win.__agentmuxList = { shown: true, cursor: true };
        const e = l.fire("keydown", { key: "Enter", target: field("search") });
        expect(e.preventDefault).not.toHaveBeenCalled();
        expect(l.lists().map((r) => r.kind)).toEqual(["open", "close"]);
    });

    it("keys in something that is not a field do nothing", () => {
        document.body.innerHTML = SIGNIN + "<div id=d></div>";
        const l = install();
        l.win.__agentmuxList = { shown: true, cursor: true };
        const e = l.fire("keydown", { key: "ArrowDown", target: field("d") });
        expect(e.preventDefault).not.toHaveBeenCalled();
        expect(l.lists()).toEqual([]);
    });
});
