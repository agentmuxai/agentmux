// @vitest-environment jsdom
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The two places that decide whether a password field is for a NEW password must agree (#47578, #6761):
// the page's listener (`signin_notice.js`, `newPasswordField`, which offers the person a strong password
// when they focus the field) and the probe an agent's `signup_probe` runs on the field it names
// (`browser_api/signup_probe.rs`, `probe_js`). Rust only checks that both hold the same two word lists;
// this runs both, as the text the host sends, on the same forms, and says where they part.
//
// `newPasswordField` is private to the listener's closure, so it is reached the way the page reaches it:
// the listener is installed on a stand-in window and sent a trusted `focusin` for the field; it answers
// with a `newpassword` report when it takes the field for a new one. The probe is cut out of its Rust
// source, with the shared `VISIBLE` and `SHOWN` snippets put in, and called on the field. jsdom has no
// layout, so a box is given to every element unless it carries `data-hidden`.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const read = (path) => readFileSync(join(here, path), "utf8");

const NOTICE = read("signin_notice.js");

function snippet(file, name) {
    const found = read(file).match(new RegExp(`const ${name}: &str = r#"([\\s\\S]*?)"#;`));
    if (!found) throw new Error(`${name} is no longer a raw string constant in ${file}: update this test`);
    return found[1];
}

// The probe's `format!` raw string, as the JavaScript the host sends.
function probeSource() {
    const found = read("../browser_api/signup_probe.rs").match(/r#"(function \(\) \{\{[\s\S]*?\}\})"#/);
    if (!found) throw new Error("probe_js is no longer one raw string in signup_probe.rs: update this test");
    return found[1]
        .replaceAll("{{", "{")
        .replaceAll("}}", "}")
        .replace("{VISIBLE}", snippet("../browser_api/signin_fill.rs", "VISIBLE"))
        .replace("{SHOWN}", snippet("../browser_api/signup_fill.rs", "SHOWN"));
}
const probeFn = new Function(`return (${probeSource()});`)();

beforeAll(() => {
    const hidden = (el) => el.hasAttribute("data-hidden");
    Element.prototype.getClientRects = function () {
        return hidden(this) ? [] : [{}];
    };
    Element.prototype.getBoundingClientRect = function () {
        return hidden(this) ? { width: 0, height: 0 } : { width: 200, height: 24 };
    };
});
afterEach(() => {
    document.body.innerHTML = "";
});

// The listener, installed on a stand-in window that keeps what it listens for and what it reports.
function listener() {
    const handlers = new Map();
    const reports = [];
    const extra = {
        addEventListener: (type, fn) => handlers.set(type, fn),
        agentmuxSignIn: (json) => reports.push(JSON.parse(json)),
    };
    const win = new Proxy(window, {
        get: (target, key) => (key in extra ? extra[key] : typeof target[key] === "function" ? target[key].bind(target) : target[key]),
        set: (target, key, value) => ((extra[key] = value), true),
    });
    new Function("window", NOTICE)(win);
    return (el) => {
        reports.length = 0;
        handlers.get("focusin")({ isTrusted: true, target: el });
        return reports.some((r) => r.kind === "newpassword");
    };
}

const field = (id) => document.getElementById(id);
const probe = (id) => probeFn.call(field(id));

// Each case: the page, and per field: is it for a new password, and does the probe call the form a change.
const CASES = [
    {
        name: "a field marked new-password",
        html: `<form><input id=u type=text name=user><input id=pw type=password autocomplete="new-password"></form>`,
        fields: { pw: { fresh: true }, u: { fresh: false } },
    },
    {
        name: "the mark is read in any case",
        html: `<form><input id=pw type=password autocomplete="NEW-PASSWORD"></form>`,
        fields: { pw: { fresh: true } },
    },
    {
        name: "two passwords, the second says confirm: the first is the new one",
        html: `<form><input id=pw type=password name=password><input id=pw2 type=password name=password_confirm></form>`,
        fields: { pw: { fresh: true }, pw2: { fresh: false } },
    },
    {
        name: "two passwords, the second says nothing of confirming",
        html: `<form><input id=pw type=password name=password><input id=pw2 type=password name=second></form>`,
        fields: { pw: { fresh: false }, pw2: { fresh: false } },
    },
    {
        name: "one password and a Create account button in its form",
        html: `<form><input id=pw type=password><button>Create account</button></form>`,
        fields: { pw: { fresh: true } },
    },
    {
        name: "one password and a Sign up submit input in its form",
        html: `<form><input id=pw type=password><input type=submit value="Sign up"></form>`,
        fields: { pw: { fresh: true } },
    },
    {
        name: "one password and a Sign in button: a sign-in",
        html: `<form><input id=pw type=password><button>Sign in</button></form>`,
        fields: { pw: { fresh: false } },
    },
    {
        name: "a form with a current-password field is a change, whatever else it holds",
        html: `<form><input id=cur type=password autocomplete="current-password"><input id=pw type=password autocomplete="new-password"><input id=pw2 type=password autocomplete="new-password"></form>`,
        fields: { cur: { fresh: false, change: true }, pw: { fresh: false, change: true }, pw2: { fresh: false, change: true } },
    },
    {
        name: "no form: the page's one password and a Create account button",
        html: `<input id=pw type=password><button>Create account</button>`,
        fields: { pw: { fresh: false } },
    },
    {
        name: "a hidden confirmation field is not counted",
        html: `<form><input id=pw type=password><input id=pw2 type=password name=confirm data-hidden><button>Join</button></form>`,
        fields: { pw: { fresh: true } },
    },
    {
        name: "a hidden current-password field does not make a change",
        html: `<form><input id=cur type=password autocomplete="current-password" data-hidden><input id=pw type=password autocomplete="new-password"></form>`,
        fields: { pw: { fresh: true } },
    },
];

describe("a field for a new password: the page's listener and the agent's probe agree", () => {
    for (const { name, html, fields } of CASES) {
        it(name, () => {
            document.body.innerHTML = html;
            const listens = listener();
            for (const [id, want] of Object.entries(fields)) {
                const isPassword = field(id).type === "password";
                const heard = listens(field(id));
                const answer = isPassword ? probe(id) : { error: "not a password field" };
                const probed = typeof answer.field === "string" && answer.field.length > 0;
                expect(probed, `${id}: the probe and the listener must agree`).toBe(heard);
                expect({ id, fresh: heard }).toEqual({ id, fresh: want.fresh });
                expect({ id, change: answer.change === true }).toEqual({ id, change: want.change === true });
            }
        });
    }

    it("the probe marks the field it takes for new, and only that one", () => {
        document.body.innerHTML = `<form><input id=pw type=password autocomplete="new-password"></form><form><input id=pw2 type=password autocomplete="new-password"></form>`;
        const first = probe("pw");
        const second = probe("pw2");
        expect(first.field).toBeTruthy();
        expect(second.field).toBeTruthy();
        expect(field("pw").hasAttribute("data-agentmux-signup")).toBe(false);
        expect(field("pw2").getAttribute("data-agentmux-signup")).toBe(second.field);
    });

    it("the probe refuses a field the person cannot see, as the listener never offers one", () => {
        document.body.innerHTML = `<form><input id=pw type=password autocomplete="new-password" data-hidden></form>`;
        expect(probe("pw").error).toMatch(/isn't visible/);
    });
});
