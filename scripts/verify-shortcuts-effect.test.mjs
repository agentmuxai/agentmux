// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { expect, it } from "vitest";
import { changed, describe, idleNoise, judge, noEffectReason, NO_EFFECT_OK, OPPOSITE } from "./verify-shortcuts-effect.mjs";

const snap = (over = {}) => ({ title: "t", tabs: "*A", panes: 2, tiles: "a@0,0,1,1", focus: "a/", dialogs: 0, menus: 0, zoom: "1/1/1", paneZoom: "1", html: 1, ...over });

it("changed names the fields that differ, in order", () => {
    expect(changed(snap(), snap({ panes: 3, tiles: "x" }))).toEqual(["panes", "tiles"]);
    expect(changed(snap(), snap())).toEqual([]);
});

it("a page that changes by itself is judged without its body hash", () => {
    expect(changed(snap(), snap({ html: 2 }))).toEqual(["html"]);
    expect(changed(snap(), snap({ html: 2 }), { noisy: true })).toEqual([]);
    expect(idleNoise(snap(), snap({ html: 2 }))).toEqual(["html"]);
});

it("a command that ran and changed nothing fails unless it has nothing to show", () => {
    expect(judge("pane:magnify", [], true)).toBe("nothing visible changed");
    expect(judge("pane:magnify", ["tiles"], true)).toBe(null);
    expect(judge("app:escape", [], true)).toBe(null);
    expect(judge("pane:magnify", [], false)).toBe(null);
});

it("every command allowed to show nothing says why", () => {
    for (const [command, why] of Object.entries(NO_EFFECT_OK)) expect(why.length, command).toBeGreaterThan(10);
});

it("describe words the cell", () => {
    expect(describe("tab:new", ["tabs", "title"], true)).toBe("changed: tabs, title");
    expect(describe("files:copy", [], true)).toMatch(/^none \(/);
    expect(describe("tab:new", [], false)).toBe("");
});

it("a numbered target that does not exist is not a missing effect", () => {
    expect(noEffectReason("tab:goto:5", { tabs: 3 })).toBeTruthy();
    expect(noEffectReason("tab:goto:2", { tabs: 3 })).toBe(null);
    expect(noEffectReason("tab:goto:last", { tabs: 1 })).toBeTruthy();
    expect(noEffectReason("tab:goto:last", { tabs: 3 })).toBe(null);
    expect(noEffectReason("pane:focus:4", { panes: 2 })).toBeTruthy();
    expect(noEffectReason("pane:focus:2", { panes: 2 })).toBe(null);
    expect(noEffectReason("pane:magnify", { panes: 2 })).toBe(null);
});

it("opposites point at each other", () => {
    for (const [a, b] of Object.entries(OPPOSITE)) expect(OPPOSITE[b], a).toBe(a);
});
