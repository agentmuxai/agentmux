// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings › Notifications & tray, pinned as a whole ahead of splitting it into parts: each scenario mounts the real
 * section over a settings object held in a signal, with stand-ins for the toggle, select, text and button controls,
 * the host's capabilities (a tray, login entries), the launcher's autostart status and the test-notification RPC, and
 * logs the section as an indented tree (every control with the props it was given) and, in order, every setting it
 * writes and every read of the login registration. The logs are snapshots recorded on the unsplit component; the
 * split must leave them byte for byte. (This pins the markup and the writes; the controls' own behaviour is tested
 * with the controls, and notifications-section.test.tsx asserts what the person finds.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    arg: undefined as unknown,
    caps: {} as Record<string, boolean>,
    autostart: { available: true, enabled: false } as { available: boolean; enabled: boolean },
    autostartFails: false,
    notifyFails: false,
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
        TextInput: (props: Record<string, any>) => {
            const { onBlur, ...rest } = props;
            return <input data-stand="TextInput" {...rest} onBlur={onBlur} />;
        },
        Button: (props: Record<string, any>) => (
            <span data-stand="Button" data-cb="onClick" onClick={() => props.onClick()}>
                {props.children}
            </span>
        ),
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
vi.mock("@/util/host-caps", () => ({ hostHas: (cap: string) => h.caps[cap] ?? false }));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        getAutostartStatus: () => {
            h.log.push("getAutostartStatus");
            return h.autostartFails ? Promise.reject(new Error("no verb")) : Promise.resolve(h.autostart);
        },
    }),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        NotifyTestCommand: () => {
            h.log.push("NotifyTestCommand");
            return h.notifyFails ? Promise.reject(new Error("no")) : Promise.resolve();
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("../settings-controls", async (orig) => {
    const actual = await orig<typeof import("../settings-controls")>();
    return {
        ...actual,
        set: (key: string, value: unknown) => h.log.push(`set ${key} ${JSON.stringify(value)}`),
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
import { NotificationsSection } from "./notifications-section";

const setSettings = (globalStore as any).__setSettings as (v: unknown) => void;
const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/notifications-section.behavior.${name}.txt`);

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
    const view = render(() => <NotificationsSection />);
    host = view.container;
}
const row = (id: string) => host.querySelector(`#setting-notifications\\.${id}`) as HTMLElement | null;
const trigger = (id: string, arg?: unknown) => {
    h.arg = arg;
    fireEvent.click(row(id)!.querySelector("[data-cb]")!);
};

const CONFIGURED = {
    "notify:os:enabled": true,
    "notify:os:when": "always",
    "notify:os:inputwaiting": false,
    "notify:os:turncompleted": false,
    "notify:os:turncompleted:external": true,
    "notify:os:turnerrored": false,
    "notify:os:agentcrashed": false,
    "notify:os:messageneedsreview": false,
    "notify:os:cloudsignedout": false,
    "notify:os:accountunavailable": false,
    "notify:os:jobfailed": false,
    "notify:os:vaultexpiring": false,
    "notify:os:summary": false,
    "notify:taskbar:attention": false,
    "notify:quiethours": "22:00-08:00",
    "notify:pause:allowattention": true,
    "notify:os:preview": "full",
    "app:showtray": false,
    "app:runinbackground": true,
    "app:startatlogin": true,
};

beforeEach(() => {
    T.length = 0;
    setSettings(undefined);
    h.caps = { tray: true, autostart: true };
    h.autostart = { available: true, enabled: false };
    h.autostartFails = false;
    h.notifyFails = false;
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("NotificationsSection, pinned", () => {
    it("shows every setting with its defaults on a host with a tray and login entries", async () => {
        mount();
        await flush();
        look("defaults");
        await snapshot("defaults");
    });

    it("shows the values the settings hold", async () => {
        setSettings(CONFIGURED);
        h.autostart = { available: true, enabled: true };
        mount();
        await flush();
        look("configured");
        await snapshot("configured");
    });

    it("hides what the master switch governs while it is off, and the tray rows a host lacks", async () => {
        setSettings({ "notify:os:enabled": false });
        mount();
        await flush();
        look("desktop notifications off");
        setSettings({ "notify:os:enabled": true });
        await flush();
        look("on again");
        h.caps = { tray: true };
        setSettings({ "notify:os:enabled": true, "x": 1 });
        cleanup();
        mount();
        await flush();
        look("a host with a tray only");
        h.caps = { autostart: true };
        cleanup();
        mount();
        await flush();
        look("a host with login entries only");
        h.caps = {};
        cleanup();
        mount();
        await flush();
        look("a host with neither");
        await snapshot("hosts");
    });

    it("reports a launcher that cannot manage a login entry, one that did not register it, and an older host without the verb", async () => {
        h.autostart = { available: false, enabled: false };
        mount();
        await flush();
        look("unavailable");
        cleanup();
        setSettings({ "app:startatlogin": true });
        h.autostart = { available: true, enabled: false };
        mount();
        await flush();
        look("on but not registered");
        cleanup();
        h.autostartFails = true;
        mount();
        await flush();
        look("the verb is missing");
        await snapshot("autostart");
    });

    it("reads the registration again two seconds after a change to start at login, and not for another setting", async () => {
        mount();
        await flush();
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        log("== an unrelated setting changes");
        setSettings({ "notify:os:summary": false });
        await vi.advanceTimersByTimeAsync(3000);
        log("== start at login turns on");
        setSettings({ "app:startatlogin": true });
        await vi.advanceTimersByTimeAsync(1999);
        look("1999 ms later");
        h.autostart = { available: true, enabled: true };
        await vi.advanceTimersByTimeAsync(1);
        await vi.advanceTimersByTimeAsync(10);
        look("2000 ms later, registered");
        await snapshot("refetch");
    });

    it("writes each toggle's setting with the opposite value, each select's with the choice, the quiet hours trimmed or null, and sends the test", async () => {
        mount();
        await flush();
        for (const id of [
            "os_enabled",
            "os_input_waiting",
            "os_turn_completed",
            "os_turn_completed_external",
            "os_turn_errored",
            "os_agent_crashed",
            "os_needs_review",
            "os_cloud_signed_out",
            "os_account_unavailable",
            "os_job_failed",
            "os_vault_expiring",
            "os_summary",
            "taskbar_attention",
            "pause_allow_attention",
            "show_tray",
            "run_in_background",
            "autostart",
        ])
            trigger(id);
        trigger("os_when", "never");
        trigger("os_preview", "none");
        const input = row("quiet_hours")!.querySelector("input")!;
        input.value = "  23:00-07:00 ";
        fireEvent.blur(input);
        input.value = "   ";
        fireEvent.blur(input);
        trigger("os_test");
        h.notifyFails = true;
        trigger("os_test");
        await flush();
        await snapshot("writes");
    });
});
