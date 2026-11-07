// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AppearanceSection (Settings › Appearance), pinned as a whole ahead of splitting it into view parts: each scenario
 * mounts the real section over stand-ins for the kit's select and text input, the settings rows and controls, the
 * Language row, the widget colors, the host's capabilities and the settings signal, whose answers the scenario
 * controls, drives the controls, and logs the section as an indented tree and, in order, every call it makes (the
 * capability it asks the host for, each setting written).
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (appearance-section.test.tsx pins the splash toggle against the real controls; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const [settings, setSettings] = createSignal<Record<string, unknown> | undefined>({});
const [hasTransparency, setHasTransparency] = createSignal(true);

vi.mock("@/app/store/global", () => ({ settingsAtom: () => settings() }));
vi.mock("@/util/host-caps", () => ({
    hostHas: (cap: string) => (log(`hostHas ${cap}`), cap === "windowTransparency" ? hasTransparency() : false),
}));
vi.mock("@/app/menu/base-menus", () => ({
    THEME_OPTIONS: [
        { id: "default", label: "Default" },
        { id: "dark", label: "Dark" },
    ],
}));
vi.mock("@/app/element/ui", () => ({
    Select: (p: { class?: string; value: string; onChange: (v: string) => void; children?: JSX.Element }) => (
        <div data-stand="Select" class={p.class} data-value={p.value} onClick={() => p.onChange("dark")}>
            {p.children}
        </div>
    ),
    TextInput: (p: {
        class?: string;
        type?: string;
        value?: string;
        placeholder?: string;
        onBlur?: (e: FocusEvent & { currentTarget: HTMLInputElement }) => void;
    }) => (
        <input
            data-stand="TextInput"
            class={p.class}
            type={p.type}
            data-value={p.value}
            placeholder={p.placeholder}
            onBlur={(e) => p.onBlur?.(e)}
        />
    ),
}));
vi.mock("../settings-controls", () => ({
    set: (key: string, value: unknown) => log(`set ${key} ${JSON.stringify(value)}`),
    SectionHeader: (p: { label: string }) => <h3 data-stand="SectionHeader">{p.label}</h3>,
    SettingRow: (p: {
        id?: string;
        indent?: boolean;
        label: string;
        description?: string;
        control: JSX.Element;
    }) => (
        <div
            data-stand="SettingRow"
            data-id={p.id}
            data-indent={String(!!p.indent)}
            data-label={p.label}
            data-description={p.description}
        >
            {p.control}
        </div>
    ),
    ToggleControl: (p: { checked: boolean; onChange: (v: boolean) => void }) => (
        <span
            data-stand="ToggleControl"
            data-checked={String(p.checked)}
            onClick={() => (log(`ToggleControl ${p.checked} click`), p.onChange(!p.checked))}
        />
    ),
    SliderControl: (p: { min: number; max: number; step: number; value: number; onChange: (v: number) => void }) => (
        <span
            data-stand="SliderControl"
            data-min={p.min}
            data-max={p.max}
            data-step={p.step}
            data-value={p.value}
            onClick={() => (log(`SliderControl ${p.value} click`), p.onChange(p.max))}
        />
    ),
    NumberControl: (p: {
        class?: string;
        min?: number;
        max?: number;
        step?: number;
        parse?: string;
        value: number;
        onChange: (v: number) => void;
    }) => (
        <span
            data-stand="NumberControl"
            class={p.class}
            data-min={p.min}
            data-max={p.max}
            data-step={p.step}
            data-parse={p.parse}
            data-value={p.value}
            onClick={() => (log(`NumberControl ${p.value} click`), p.onChange(7))}
        />
    ),
}));
vi.mock("./language-row", () => ({
    LANGUAGE_SETTINGS: { language: { id: "appearance.language", label: "Language", section: "appearance", keywords: [] } },
    LanguageRow: () => <div data-stand="LanguageRow" />,
}));
vi.mock("./widget-colors", () => ({
    WidgetColorsSettings: (p: { id: string; label: string; description: string }) => (
        <div data-stand="WidgetColorsSettings" data-id={p.id} data-label={p.label} data-description={p.description} />
    ),
}));

import { AppearanceSection } from "./appearance-section";

const SNAP = (name: string) => `./__snapshots__/appearance-section.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|value|placeholder)$/;

/** The section as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
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
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const control = (id: string) => host.querySelector<HTMLElement>(`[data-id='${id}'] [data-stand]`)!;
const click = (id: string) => fireEvent.click(control(id));
const blurWith = (id: string, value: string) => {
    const input = control(id) as HTMLInputElement;
    input.value = value;
    fireEvent.blur(input);
};

function mount() {
    const view = render(() => <AppearanceSection />);
    host = view.container;
    return view;
}

beforeEach(() => {
    T.length = 0;
    setSettings({});
    setHasTransparency(true);
});
afterEach(() => {
    cleanup();
});

describe("AppearanceSection, pinned", () => {
    it("shows every row with its default when nothing is set: the theme, the Language row, the splash on, the transparency off with its two sub-rows out, the pane gap, the background color, the hover-magnify settings and the widget colors", () => {
        mount();
        look("defaults");
        return snapshot("defaults");
    });

    it("leaves the transparency rows out on a host without the capability, even with transparency set, and brings them in when it has it", () => {
        setSettings({ "window:transparent": true });
        setHasTransparency(false);
        mount();
        look("no capability");
        setHasTransparency(true);
        look("capability");
        setSettings(undefined);
        look("no settings");
        return snapshot("capability");
    });

    it("reads each row's value from the settings: the theme, the splash off, transparency on with its opacity and blur, the pane gap, the background color and the hover-magnify numbers", () => {
        setSettings({
            "window:theme": "dark",
            "splash:disabled": true,
            "window:transparent": true,
            "window:opacity": 0.5,
            "window:blur": true,
            "window:tilegapsize": 8,
            "window:bgcolor": "#222222",
            "window:magnifiedblockopacity": 0.6,
            "window:magnifiedblocksize": 2,
            "window:magnifiedblockblurprimarypx": 4,
            "window:magnifiedblockblursecondarypx": 9,
        });
        mount();
        look("stored values");
        setSettings({ "window:transparent": true, "window:opacity": 0, "window:tilegapsize": 0 });
        look("zero values");
        return snapshot("stored");
    });

    it("writes each row's setting back: the theme, the splash inverted, transparency, opacity and blur, the pane gap, the background color (blank as none) and the four hover-magnify numbers", () => {
        mount();
        click("appearance.theme");
        click("appearance.splash");
        click("appearance.transparency");
        setSettings({ "window:transparent": true });
        click("appearance.opacity");
        click("appearance.blur");
        click("appearance.pane_gap");
        blurWith("appearance.bg_color", "#abcdef");
        blurWith("appearance.bg_color", "");
        click("appearance.magnified_opacity");
        click("appearance.magnified_size");
        click("appearance.magnified_blur_primary");
        click("appearance.magnified_blur_secondary");
        setSettings({ "window:transparent": false, "splash:disabled": true });
        click("appearance.transparency");
        click("appearance.splash");
        look("after the writes");
        return snapshot("writes");
    });
});
