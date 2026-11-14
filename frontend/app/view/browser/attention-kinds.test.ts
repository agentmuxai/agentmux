// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { attentionLook, chooserNoticeText, DIALOG_MESSAGE_MAX, dialogMessage, noticeText } from "./attention-kinds";

const a = (over: Partial<BrowserAttention>): BrowserAttention => ({ id: "1", kind: "approval", ...over });

describe("attentionLook", () => {
    it("keeps each existing kind's buttons", () => {
        expect(attentionLook(a({ kind: "handoff" }))).toMatchObject({ yes: "Done", answer: "done", no: "Cancel" });
        expect(attentionLook(a({ kind: "navigation" }))).toMatchObject({ yes: "Allow", no: "Block", agent: false });
        expect(attentionLook(a({ kind: "bypass" }))).toMatchObject({ yes: "Allow auto-approve", no: "No", tone: "danger" });
        expect(attentionLook(a({ kind: "approval" }))).toMatchObject({ yes: "Approve", answer: "approve" });
    });

    it("shows the host's own line on a local notice, with nothing to go back from", () => {
        const look = attentionLook(a({ kind: "notice", local: true, message: "Couldn't copy the link." }));
        expect(look).toMatchObject({ text: "Couldn't copy the link.", no: "Dismiss", agent: false });
        expect(look.back).toBeFalsy();
        expect(look.showUrl).toBeFalsy();
    });

    it("asks whether another agent may drive the pane, with Not now as the way out", () => {
        expect(attentionLook(a({ kind: "take" }))).toMatchObject({ yes: "Allow", answer: "approve", no: "Not now", agent: true });
    });

    it("answers a page's dialog in the browser's own words", () => {
        expect(attentionLook(a({ kind: "dialog", dialogKind: "confirm" }))).toMatchObject({ yes: "OK", no: "Cancel" });
        expect(attentionLook(a({ kind: "dialog", dialogKind: "beforeunload" }))).toMatchObject({
            yes: "Leave",
            no: "Stay",
            answer: "approve",
        });
    });

    it("shows the address a navigation question is about", () => {
        expect(attentionLook(a({ kind: "navigation" }))).toMatchObject({ showUrl: true, icon: "fa-hand" });
        expect(attentionLook(a({ kind: "handoff" })).showUrl).toBeFalsy();
    });

    it("offers Back and Dismiss on a notice, and Allow only when srv can still add the site", () => {
        const withAllow = attentionLook(a({ kind: "notice", allow: true, origin: "https://accounts.google.com" }));
        expect(withAllow).toMatchObject({ agent: false, back: true, yes: "Allow", answer: "approve", no: "Dismiss" });
        expect(attentionLook(a({ kind: "notice", allow: false })).yes).toBeUndefined();
        expect(attentionLook(a({ kind: "notice" })).icon).toBe("fa-circle-info");
    });
});

describe("noticeText", () => {
    it("says where the pane is limited to, where the page tried to go, and why it stopped", () => {
        const n = a({
            kind: "notice",
            sites: ["https://github.com"],
            origin: "https://accounts.google.com",
            url: "https://accounts.google.com/o/oauth2",
            why: "You blocked it.",
        });
        expect(noticeText(n)).toBe(
            "Stopped: this pane is limited to github.com, and the page tried to go to accounts.google.com. You blocked it.",
        );
        expect(attentionLook(n).text).toBe(noticeText(n));
    });

    it("falls back to the address when it has no site, and names a popup", () => {
        expect(noticeText(a({ kind: "notice", url: "data:,x", sites: [] }))).toBe(
            "Stopped: this pane is limited to some sites, and the page tried to go to data:,x.",
        );
        expect(noticeText(a({ kind: "notice", popup: true, origin: "https://b.example", sites: ["https://a.example"] }))).toContain(
            "tried to open a window at b.example",
        );
    });
});

describe("chooserNoticeText", () => {
    it("tells the person a cancelled file chooser is theirs after Take over, and offers only Dismiss", () => {
        const n = a({ kind: "notice", chooser: "choose a file" });
        expect(chooserNoticeText(n)).toBe(
            "The page asked to choose a file, but an agent drives this pane, so AgentMux cancelled it. Use Take over, then click again to choose a file yourself.",
        );
        expect(attentionLook(n)).toMatchObject({ agent: false, no: "Dismiss", text: chooserNoticeText(n) });
        expect(attentionLook(n).yes).toBeUndefined();
        expect(attentionLook(n).back).toBeFalsy();
        expect(attentionLook(n).showUrl).toBeFalsy();
    });

    it("names several files and a folder in their own words", () => {
        expect(chooserNoticeText(a({ kind: "notice", chooser: "choose several files" }))).toContain("to choose files,");
        expect(chooserNoticeText(a({ kind: "notice", chooser: "choose a folder" }))).toContain("click again to choose a folder yourself");
    });
});

describe("dialogMessage", () => {
    it("shows the page's words trimmed and clipped", () => {
        expect(dialogMessage("  Are you sure?  ")).toBe("Are you sure?");
        expect(dialogMessage(undefined)).toBe("");
        const long = dialogMessage("x".repeat(DIALOG_MESSAGE_MAX + 10));
        expect(long.length).toBe(DIALOG_MESSAGE_MAX + 1);
        expect(long.endsWith("…")).toBe(true);
    });
});
