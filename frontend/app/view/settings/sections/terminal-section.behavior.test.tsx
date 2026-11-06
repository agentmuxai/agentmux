// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings › Terminal, pinned as a whole ahead of splitting it into parts: each scenario mounts the real section over a
 * settings object and a config of terminal themes held in signals, with stand-ins for the number, slider, toggle, select
 * and text controls and the default-shell setting, and logs the section as an indented tree (every control with the
 * props it was given) and, in order, every setting it writes. The logs are snapshots recorded on the unsplit component;
 * the split must leave them byte for byte. (This pins the markup and the writes; the controls' own behaviour is tested
 * with the controls.)
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
    const [config, setConfig] = createSignal<unknown>(undefined);
    return { settingsAtom: settings, fullConfigAtom: config, __setSettings: setSettings, __setConfig: setConfig };
});
vi.mock("@/app/element/ui", async (orig) => {
    const actual = await orig<typeof import("@/app/element/ui")>();
    return {
        ...actual,
        TextInput: (props: Record<string, any>) => {
            const { onBlur, ...rest } = props;
            return <input data-stand="TextInput" {...rest} onBlur={onBlur} />;
        },
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
vi.mock("./default-shell", () => ({ DefaultShellSetting: () => <div data-stand="DefaultShellSetting" /> }));
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
        NumberControl: valueControl("NumberControl"),
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
import { TerminalSection } from "./terminal-section";

const setSettings = (globalStore as any).__setSettings as (v: unknown) => void;
const setConfig = (globalStore as any).__setConfig as (v: unknown) => void;
const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/terminal-section.behavior.${name}.txt`);

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
    const view = render(() => <TerminalSection />);
    host = view.container;
}
const row = (id: string) => host.querySelector(`#setting-terminal\\.${id}`) as HTMLElement;
const trigger = (id: string, arg?: unknown) => {
    h.arg = arg;
    fireEvent.click(row(id).querySelector("[data-cb]")!);
};

const THEMES = {
    termthemes: {
        solar: { "display:name": "Solarized", "display:order": 2 },
        plain: { "display:order": 3 },
        dark: { "display:name": "Dark", "display:order": 1 },
        unordered: { "display:name": "Unordered" },
    },
};
const CONFIGURED = {
    "term:fontsize": 18,
    "term:fontfamily": "Fira Code, monospace",
    "term:theme": "dark",
    "term:scrollback": 50000,
    "term:copyonselect": true,
    "term:shiftenternewline": true,
    "term:durable": false,
    "conn:helper": "always",
    "term:allowbracketedpaste": false,
    "term:transparency": 0.25,
    "term:scrollsensitivity": 2.5,
    "term:showstatsbadge": false,
    "term:predictiveecho": true,
    "term:predictiveecho:thresholdms": 120,
    "term:agentmaxruntimehours": 8,
    "term:agentidletimeoutmins": 45,
};

beforeEach(() => {
    T.length = 0;
    setSettings(undefined);
    setConfig(undefined);
});
afterEach(() => {
    cleanup();
});

describe("TerminalSection, pinned", () => {
    it("shows every setting with its defaults when nothing is set", async () => {
        mount();
        await flush();
        look("defaults, no themes");
        await snapshot("defaults");
    });

    it("shows the values the settings hold, and the themes in their order", async () => {
        setConfig(THEMES);
        setSettings(CONFIGURED);
        mount();
        await flush();
        look("configured");
        await snapshot("configured");
    });

    it("follows the settings and the themes while it is open", async () => {
        mount();
        await flush();
        setConfig(THEMES);
        setSettings({ "term:durable": true, "conn:helper": "never", "term:predictiveecho": false, "term:theme": "solar" });
        await flush();
        look("after a change: echo off, so no threshold row");
        setSettings({ "term:durable": null, "conn:helper": "", "term:showstatsbadge": true });
        await flush();
        look("durable and helper back to their defaults");
        setConfig({ termthemes: {} });
        await flush();
        look("no themes");
        await snapshot("follows");
    });

    it("writes each toggle's setting with the opposite value", async () => {
        mount();
        await flush();
        for (const id of ["copy_on_select", "shift_enter_newline", "bracketed_paste", "cpu_mem_badge", "predictive_echo"]) trigger(id);
        await flush();
        await snapshot("toggles");
    });

    it("writes each number, the slider and the font family as they are given", async () => {
        mount();
        await flush();
        trigger("font_size", 20);
        trigger("scrollback", 12000);
        trigger("transparency", 0.7);
        trigger("scroll_sensitivity", 3.3);
        trigger("predictive_echo_threshold", 90);
        trigger("agent_max_runtime", 2.5);
        trigger("agent_idle_timeout", 30);
        const family = row("font_family").querySelector("input") as HTMLInputElement;
        family.value = "Menlo";
        fireEvent.blur(family);
        family.value = "";
        fireEvent.blur(family);
        await flush();
        await snapshot("numbers");
    });

    it("writes the selects, and clears the ones whose default is to ask or to decide", async () => {
        setConfig(THEMES);
        mount();
        await flush();
        trigger("theme", "solar");
        trigger("theme", "");
        trigger("durable_ssh", "on");
        trigger("durable_ssh", "off");
        trigger("durable_ssh", "auto");
        trigger("helper_install", "always");
        trigger("helper_install", "never");
        trigger("helper_install", "ask");
        await flush();
        await snapshot("selects");
    });
});
