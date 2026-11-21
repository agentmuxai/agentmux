// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Maintenance section's states, table-driven over maintenance-table.ts: every definition has a fixture here (a new
 * one without one fails), gives the message and the buttons the fixture says, and is drawn the same way as every other:
 * the message first, its buttons after it in the same order, the glyph out of the screen reader's way, the text once.
 * The stylesheet's one rule (the buttons wrap under the message when they do not fit beside it) is asserted on the source.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/store/global", () => ({ getSettingsKeyAtom: () => () => undefined, openLink: vi.fn() }));
vi.mock("@/util/app-api", () => ({ getApi: () => undefined }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: () => {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { acceptAppUpdate } from "@/store/app-update";
import { MessageRows } from "./maintenance-rows";
import type { MaintenanceState } from "./maintenance-state";
import { ALL_ROWS, type RowDef } from "./maintenance-table";

afterEach(cleanup);

type Fix = Record<string, unknown>;

/** A section state with nothing going on; a fixture overrides what its state reads. */
function state(over: Fix = {}): MaintenanceState {
    const none = () => null;
    return {
        updaterStatus: () => "up-to-date",
        updaterVersion: () => "0.60.36",
        installFirst: () => undefined,
        storeBelowFloor: () => false,
        storeOffers: () => true,
        opensDownloadPage: () => false,
        download: none,
        percent: () => 0,
        release: () => ({ url: "https://example.test/release" }),
        downloadBlocked: none,
        startError: none,
        standing: () => "v0.60.35 is up to date (checked 01:30 PM)",
        checking: () => false,
        checkError: none,
        storeLagging: () => false,
        storeBusy: () => false,
        storeNote: none,
        storeDownloaded: none,
        restartAsking: () => false,
        restartWho: none,
        restarting: () => false,
        handleInstallUpdate: vi.fn(),
        handleCheck: vi.fn(),
        reveal: vi.fn(),
        runUpgrade: vi.fn(),
        cancelRestart: vi.fn(),
        openStorePage: vi.fn(),
        askRestart: vi.fn(),
        cancelAskRestart: vi.fn(),
        restartForStore: vi.fn(),
        ...over,
    } as unknown as MaintenanceState;
}

const dl = (over: Fix) => () => ({ state: "downloading", received: 0, total: 0, waiting_for: 0, stopping: [], ...over });
const LONG = "an unbroken token ".repeat(1) + "C:/Users/someone/.agentmux/channels/stable/versions/0.60.36/data/downloads/agentmux-0.60.36-windows-x64-setup.exe.part";

/** What each definition shows for a state it applies to: the messages, and the buttons under the first of them. */
const FIXTURES: Record<string, { m: Fix; texts: string[]; buttons: string[] }> = {
    available: { m: { updaterStatus: () => "available" }, texts: ["v0.60.36 is out"], buttons: ["What's new", "Download"] },
    downloading: { m: { updaterStatus: () => "downloading", download: dl({}), percent: () => 41 }, texts: ["Downloading v0.60.36… 41%"], buttons: [] },
    staged: {
        m: { updaterStatus: () => "ready", download: dl({ state: "ready", folder: "C:/apps/x", installer: "C:/apps/x/setup.exe" }) },
        texts: ["v0.60.36 is ready"],
        buttons: ["Show folder", "Show installer", "Upgrade & restart"],
    },
    waiting: { m: { updaterStatus: () => "ready", download: dl({ state: "waiting", waiting_for: 2 }) }, texts: ["Waiting for 2 agents to finish…"], buttons: ["Restart now", "Cancel"] },
    restarting: { m: { updaterStatus: () => "ready", download: dl({ state: "restarting" }) }, texts: ["Restarting into v0.60.36…"], buttons: [] },
    "host-ready": { m: { updaterStatus: () => "ready" }, texts: ["v0.60.36 ready"], buttons: ["Restart"] },
    failed: { m: { updaterStatus: () => "error", download: dl({ state: "failed", error: LONG }) }, texts: [`Download failed: ${LONG}`], buttons: ["Try again", "Release page"] },
    stopping: {
        m: { updaterStatus: () => "ready", download: dl({ state: "waiting", stopping: ["a build in agent x", "a test run in agent y"] }) },
        texts: ["The restart also stops what agents left running:", "a build in agent x", "a test run in agent y"],
        buttons: [],
    },
    "ready-error": { m: { updaterStatus: () => "ready", download: dl({ state: "ready", error: "the folder could not be opened" }) }, texts: ["the folder could not be opened"], buttons: [] },
    "install-first": { m: { installFirst: () => "0.60.20" }, texts: ["This version is older than v0.60.36 upgrades from: install v0.60.20 first, then upgrade."], buttons: [] },
    "store-below-floor": {
        m: { storeBelowFloor: () => true, installFirst: () => "0.60.20" },
        texts: [expect.stringContaining("The Microsoft Store can only install the latest version") as unknown as string],
        buttons: ["How to move off the Store build"],
    },
    "download-blocked": { m: { downloadBlocked: () => "GitHub is limiting the check; try again later." }, texts: ["GitHub is limiting the check; try again later."], buttons: [] },
    "start-error": { m: { startError: () => LONG }, texts: [LONG], buttons: [] },
    store: { m: { storeNote: () => "The Store is slow today." }, texts: ["The Store is slow today."], buttons: [] },
    check: { m: {}, texts: ["v0.60.35 is up to date (checked 01:30 PM)"], buttons: ["Check again"] },
};

const defs = (): [string, RowDef][] => ALL_ROWS.map((def) => [def.id, def]);

describe("every state of the Maintenance section", () => {
    it("has a fixture here, so a new state cannot be added without its layout test", () => {
        expect(ALL_ROWS.map((d) => d.id).sort()).toEqual(Object.keys(FIXTURES).sort());
    });

    it.each(defs())("%s: says its message and offers its buttons", (id, def) => {
        const { m, texts, buttons } = FIXTURES[id];
        const rows = def.rows(state(m));
        expect(rows.map((r) => r.text)).toEqual(texts);
        expect(rows[0].actions.map((a) => a.label)).toEqual(buttons);
        for (const row of rows) expect(row.actions.length).toBeLessThanOrEqual(3);
    });

    it.each(defs())("%s: shows nothing when it does not apply", (id, def) => {
        if (id === "check") return; // the standing line is always there
        expect(def.rows(state())).toEqual([]);
    });

    it.each(defs())("%s: is drawn as the message, then its buttons, each read once", (id, def) => {
        const { m, texts, buttons } = FIXTURES[id];
        const host = render(() => <MessageRows m={state(m)} defs={[def]} />).container;
        const rows = [...host.querySelectorAll(".maintenance-row")];
        expect(rows).toHaveLength(texts.length);
        rows.forEach((row) => {
            expect(row.children[0].classList.contains("maintenance-message")).toBe(true);
            const message = row.querySelector(".maintenance-message")!;
            const icon = message.querySelector(".maintenance-icon");
            if (icon) expect(icon.getAttribute("aria-hidden")).toBe("true");
            expect(message.querySelectorAll(".maintenance-row-text")).toHaveLength(1);
            const actions = row.querySelector(".maintenance-actions");
            if (actions) expect(row.children[1]).toBe(actions);
        });
        const shown = [...rows[0].querySelectorAll(".maintenance-actions button")].map((b) => b.textContent?.trim());
        expect(shown).toEqual(buttons);
        // The message text is in the document once, whatever the number of buttons.
        const text = rows[0].querySelector(".maintenance-row-text")!.textContent!;
        expect(host.textContent!.split(text).length - 1).toBe(1);
    });

    // A decided restart is sent to the launcher as confirmed and opens no dialog of ours (srv-core update/restart.rs); only a
    // .deb or .rpm install still has the system's own permission prompt. The row says which, from the install kind srv reports.
    it.each([
        ["windows-setup", "Restarting into v0.60.36…"],
        ["windows-portable", "Restarting into v0.60.36…"],
        ["macos-app", "Restarting into v0.60.36…"],
        ["linux-appimage", "Restarting into v0.60.36…"],
        ["linux-portable", "Restarting into v0.60.36…"],
        ["unknown", "Restarting into v0.60.36…"],
        ["linux-package", "Restarting into v0.60.36… Your system may ask for your password to install it; refusing cancels the restart."],
    ])("restarting on a %s install says: %s", (install, text) => {
        acceptAppUpdate({ state: "available", current: "0.60.35", latest: "0.60.36", checked_ms: 1, install, can_download: true });
        const restarting = ALL_ROWS.find((d) => d.id === "restarting")!;
        const rows = restarting.rows(state(FIXTURES.restarting.m));
        expect(rows.map((r) => r.text)).toEqual([text]);
        expect(rows[0].actions).toEqual([]);
        expect(text).not.toMatch(/dialog/i);
    });

    it("a row with no buttons has no empty button area", () => {
        const host = render(() => <MessageRows m={state(FIXTURES.restarting.m)} defs={ALL_ROWS.filter((d) => d.id === "restarting")} />).container;
        expect(host.querySelector(".maintenance-actions")).toBeNull();
    });
});

describe("the stylesheet's one rule", () => {
    const scss = readFileSync(join(process.cwd(), "frontend/app/statusbar/_maintenance-section.scss"), "utf8");
    const block = (selector: string): string => scss.slice(scss.indexOf(`${selector} {`), scss.indexOf("}", scss.indexOf(`${selector} {`)));

    it("lets a row wrap its buttons under the message, never squeezing the message beside them", () => {
        expect(block(".maintenance-row")).toContain("flex-wrap: wrap");
        expect(block(".maintenance-row")).not.toContain("nowrap");
        expect(block(".maintenance-message")).toContain("min-width: 0");
    });

    it("wraps more buttons than fit onto a second line, and a long unbroken token onto several", () => {
        expect(block(".maintenance-actions")).toContain("flex-wrap: wrap");
        expect(block(".maintenance-row-text")).toContain("overflow-wrap: anywhere");
        expect(block(".maintenance-row-text")).toContain("white-space: normal");
    });
});
