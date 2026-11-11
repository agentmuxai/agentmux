// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleProviderInstructionsSection (the bundle form's per-provider instruction overrides), pinned as a whole ahead of
 * splitting it into a state object and views: each scenario mounts the real section over a stand-in for the draft's
 * raw JSON string (which the scenario can echo back or change from outside), with stand-ins for the kit's button, text
 * input and text area, the context menu, the provider catalog and the focus helper, types, adds and removes rows, and
 * logs the section as an indented tree and, in order, every call it makes (each string it hands its parent, each
 * button pressed, each context menu, the focus it asks for). The parsing, serializing and key checks
 * (bundle-model.ts) are the real ones.
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: () => log("showTextInputContextMenu") }));
vi.mock("@/app/view/agent/providers/catalog", () => ({ PROVIDERS: { zeta: {}, alpha: {}, mid: {} } }));
vi.mock("@/util/focusutil", () => ({
    focusWhenRendered: (find: () => HTMLInputElement | null | undefined) => {
        const el = find();
        log(`focusWhenRendered ${el ? `input ${JSON.stringify(el.dataset.value)}` : "nothing"}`);
    },
}));
vi.mock("@/app/element/ui", () => ({
    Button: (p: { class?: string; icon?: string; title?: string; onClick?: (e: MouseEvent) => void; children?: JSX.Element }) => (
        <span
            data-stand="Button"
            class={p.class}
            data-icon={p.icon}
            data-title={p.title}
            onClick={(e) => (log(`Button ${JSON.stringify(p.children)} click`), p.onClick?.(e))}
        >
            {p.children}
        </span>
    ),
    TextInput: (p: any) => (
        <input
            data-stand="TextInput"
            data-variant={p.variant}
            class={p.class}
            list={p.list}
            placeholder={p.placeholder}
            data-value={p.value}
            onInput={(e) => p.onInput(e)}
            onContextMenu={(e) => p.onContextMenu(e)}
        />
    ),
    TextArea: (p: any) => (
        <textarea
            data-stand="TextArea"
            data-variant={p.variant}
            class={p.class}
            rows={p.rows}
            placeholder={p.placeholder}
            data-value={p.value}
            onInput={(e) => p.onInput(e)}
            onContextMenu={(e) => p.onContextMenu(e)}
        />
    ),
}));

import { BundleProviderInstructionsSection } from "./BundleProviderInstructionsSection";

const SNAP = (name: string) => `./__snapshots__/BundleProviderInstructionsSection.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|list|placeholder|rows|value)$/;

/** The section as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").replace(/\s+/g, " ").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
let setValue: (v: string) => void;
let echo = true;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const inputs = () => [...host.querySelectorAll<HTMLInputElement>("[data-stand='TextInput']")];
const areas = () => [...host.querySelectorAll<HTMLTextAreaElement>("[data-stand='TextArea']")];
const typeKey = (i: number, v: string) => {
    inputs()[i].value = v;
    fireEvent.input(inputs()[i]);
};
const typeText = (i: number, v: string) => {
    areas()[i].value = v;
    fireEvent.input(areas()[i]);
};
const press = (label: string, nth = 0) => {
    const found = [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")].filter((b) => b.textContent?.trim() === label);
    fireEvent.click(found[nth]);
};

function mount(initial: string) {
    const [value, sv] = createSignal(initial);
    setValue = sv;
    const view = render(
        () => (
            <BundleProviderInstructionsSection
                value={value()}
                onChange={(raw) => {
                    log(`onChange ${raw}`);
                    if (echo) sv(raw);
                }}
            />
        ),
        { container: document.body.appendChild(document.createElement("div")) }
    );
    host = view.container;
}

beforeEach(() => {
    T.length = 0;
    echo = true;
});
afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
});

describe("BundleProviderInstructionsSection, pinned", () => {
    it("shows the known providers in a sorted list and only the add button for an empty value, adds a row with focus on its key, serializes each edit out (a blank key is not saved until named, content is never trimmed), and removes a row", () => {
        mount("");
        look("empty");
        press("Add provider override");
        look("a blank row");
        typeKey(0, "  claude ");
        typeText(0, " be brief \n");
        look("named and filled");
        press("Add provider override");
        typeKey(1, "codex");
        typeText(1, "be thorough");
        look("two rows");
        fireEvent.contextMenu(inputs()[0]);
        fireEvent.contextMenu(areas()[0]);
        press("Remove", 0);
        look("the first removed");
        press("Remove");
        look("none left");
        return snapshot("edit");
    });

    it("lists the rows of the stored value in provider order, each with its own warning: a key the export would skip (each reason, with the saved-but-skipped note) and a blank one (not saved), and the keys that collide on one export path", () => {
        mount(
            JSON.stringify({
                "b-claude": "be brief",
                "a/b": "first",
                "a\\b": "second",
                "/abs": "x",
                "has:colon": "x",
                "../up": "x",
                ".": "x",
                plain: "fine",
            })
        );
        look("a mixed value");
        press("Add provider override");
        look("with a blank row");
        return snapshot("warnings");
    });

    it("refuses to edit a stored value that is not a JSON object of text (invalid JSON, an array, a non-text value) and says the value is preserved, and edits again when the value becomes readable", () => {
        mount("{not json");
        look("invalid JSON");
        setValue("[1,2]");
        look("an array");
        setValue('{"claude": 5}');
        look("a non-text value");
        setValue("null");
        look("null");
        setValue('{"claude":"ok"}');
        look("readable again");
        return snapshot("malformed");
    });

    it("re-seeds its rows when the draft changes from outside (another bundle), but not on the echo of its own last edit, so a row's input stays when the key is blank and the parent echoes back", () => {
        mount('{"claude":"one"}');
        const first = inputs()[0];
        typeKey(0, "claude2");
        log(`same input element after an echoed edit: ${inputs()[0] === first}`);
        typeKey(0, "");
        look("key blanked, the echo drops the blank row from the string but not from the form");
        log(`same input element: ${inputs()[0] === first}`);
        setValue('{"other":"two","zed":"three"}');
        look("another bundle");
        setValue('{"other":"two","zed":"three"}');
        look("the same value again");
        echo = false;
        typeKey(0, "other2");
        look("edited while the parent does not echo");
        setValue('{"other2":"two","zed":"three"}');
        look("the parent catches up");
        return snapshot("reseed");
    });
});
