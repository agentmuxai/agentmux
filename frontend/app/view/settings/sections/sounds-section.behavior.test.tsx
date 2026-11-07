// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings › Sounds, pinned as a whole ahead of splitting it into parts: each scenario mounts the real section over a
 * settings object held in a signal, with stand-ins for the slider, toggle and select controls, and logs the section as
 * an indented tree (every control with the props it was given) and, in order, every setting it writes. The logs are
 * snapshots recorded on the unsplit component; the split must leave them byte for byte. (This pins the markup and the
 * writes; the controls' own behaviour is tested with the controls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    arg: undefined as unknown,
}));

vi.mock("@/app/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [settings, setSettings] = createSignal<unknown>(undefined);
    return { settingsAtom: settings, __setSettings: setSettings };
});
vi.mock("@/app/element/ui", async (orig) => {
    const actual = await orig<typeof import("@/app/element/ui")>();
    return {
        ...actual,
        Select: (props: Record<string, any>) => (
            <div data-stand="Select" data-props={describeProps(props, ["onChange", "children"])}>
                <span data-cb="onChange" onClick={() => props.onChange(h.arg)}>
                    change
                </span>
                {props.children}
            </div>
        ),
    };
});
const describeProps = (props: Record<string, unknown>, skip: string[]) => {
    const out: Record<string, string> = {};
    for (const k of Object.keys(props).sort()) {
        if (skip.includes(k)) continue;
        out[k] = typeof props[k] === "function" ? "<fn>" : (JSON.stringify(props[k]) ?? "undefined");
    }
    return JSON.stringify(out);
};
vi.mock("../settings-controls", async (orig) => {
    const actual = await orig<typeof import("../settings-controls")>();
    const valueControl = (name: string) => (props: Record<string, any>) => (
        <div data-stand={name} data-props={describeProps(props, ["onChange"])}>
            <span data-cb="onChange" onClick={() => props.onChange(h.arg)}>
                change
            </span>
        </div>
    );
    return {
        ...actual,
        set: (key: string, value: unknown) => h.log.push(`set ${key} ${JSON.stringify(value)}`),
        SliderControl: valueControl("SliderControl"),
        ToggleControl: (props: { checked: boolean; onChange: (v: boolean) => void }) => (
            <div data-stand="ToggleControl" data-checked={String(props.checked)}>
                <span data-cb="onChange" onClick={() => props.onChange(!props.checked)}>
                    toggle
                </span>
            </div>
        ),
    };
});

import * as globalStore from "@/app/store/global";
import { SoundsSection } from "./sounds-section";

const setSettings = (globalStore as any).__setSettings as (v: unknown) => void;
const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/sounds-section.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|id|placeholder|min|max|step|value|selected)$/;
/** The section as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim().replace(/\s+/g, " ");
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    if (el instanceof HTMLInputElement) attrs.push(`prop-value=${JSON.stringify(el.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    // The kit numbers its fields as they are made, so the count depends on the tests that ran before.
    log(dump(host).join("\n").replace(/-cl-\d+/g, "-cl-N"));
};
const flush = async () => {
    for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
};
function mount() {
    const view = render(() => <SoundsSection />);
    host = view.container;
}
const row = (id: string) => host.querySelector(`#setting-sounds\\.${id}`) as HTMLElement;
const trigger = (id: string, arg?: unknown) => {
    h.arg = arg;
    fireEvent.click(row(id).querySelector("[data-cb]")!);
};

const CONFIGURED = {
    "notify:sounds:volume": 0.4,
    "notify:sounds:suppresswhenfocused": false,
    "notify:tooltones:flash": false,
    "notify:sound:agent.turn.complete": false,
    "notify:sound:agent.turn.error": false,
    "notify:sound:agent.turn.interrupted": false,
    "notify:sound:agent.message.accepted": false,
    "notify:sound:agent.message.rejected": false,
    "notify:tooltones:volume": 0.15,
    "notify:tooltones:scope": "focused",
    "notify:sounds:waiting:volume": 0.6,
    "notify:waiting:browser": false,
    "notify:sound:agent.shutdown.pending": false,
};

beforeEach(() => {
    T.length = 0;
    setSettings(undefined);
});
afterEach(() => {
    cleanup();
});

describe("SoundsSection, pinned", () => {
    it("shows every setting with its defaults when nothing is set", async () => {
        mount();
        await flush();
        look("defaults");
        await snapshot("defaults");
    });

    it("shows the values the settings hold", async () => {
        setSettings(CONFIGURED);
        mount();
        await flush();
        look("configured");
        await snapshot("configured");
    });

    it("hides what each master switch governs while it is off, and follows the settings while it is open", async () => {
        setSettings({ "notify:sounds:enabled": false, "notify:tooltones:enabled": false, "notify:sound:agent.waiting.for.input": false });
        mount();
        await flush();
        look("all three off");
        setSettings({ "notify:sounds:enabled": true, "notify:tooltones:enabled": false, "notify:sound:agent.waiting.for.input": false });
        await flush();
        look("notification sounds back on");
        setSettings({ "notify:sounds:enabled": false, "notify:tooltones:enabled": true, "notify:sound:agent.waiting.for.input": true });
        await flush();
        look("tool tones and waiting on");
        setSettings(undefined);
        await flush();
        look("settings cleared");
        await snapshot("switches");
    });

    it("writes each toggle's setting with the opposite value, each slider's with the value moved to, and the scope with the choice", async () => {
        mount();
        await flush();
        for (const id of [
            "notifications_enabled",
            "suppress_when_focused",
            "tool_tones_flash",
            "turn_complete",
            "turn_error",
            "turn_interrupted",
            "message_accepted",
            "message_rejected",
            "tool_tones_enabled",
            "waiting_tone_enabled",
            "waiting_browser",
            "shutdown_tone_enabled",
        ])
            trigger(id);
        for (const id of ["volume", "tool_tones_volume", "waiting_tone_volume"]) trigger(id, 0.7);
        trigger("tool_tones_scope", "focused");
        await flush();
        await snapshot("writes");
    });
});
