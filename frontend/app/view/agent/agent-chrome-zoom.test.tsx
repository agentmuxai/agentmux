// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The agent pane's controls follow the app's chrome zoom; the conversation keeps the pane's. The scale itself is CSS
 * (a layout engine is needed to see it), so these pin what the CSS and the code have to agree on: the rule that
 * binds the siblings after the conversation, the selector a Ctrl+wheel uses to tell them from the conversation, and the
 * pieces that read the old zoom.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { AGENT_PANE_CHROME_SELECTOR, isOverChrome } from "@/app/store/chrome-zoom-target";

const read = (rel: string) => readFileSync(resolve(__dirname, rel), "utf8").replace(/\r\n/g, "\n");

/** A pane as the layout draws it: the zoomed column holds the conversation and, after it, the controls. */
const PANE = `
  <div class="agent-view">
    <div class="agent-top-band" data-zoom-scope="chrome"><b id="band"></b></div>
    <div class="agent-view-row">
      <div class="agent-view-zoomed">
        <div class="btw"><b id="btw"></b></div>
        <div class="agent-document-scroll-region"><b id="conversation"></b></div>
        <div class="agent-desktop-bar"><b id="desktop-bar"></b></div>
        <div class="agent-auth-panel-card"><b id="sign-in"></b></div>
        <div class="agent-auth-notice"><b id="sign-in-notice"></b></div>
        <div class="agent-request-stack"><b id="request"></b></div>
        <div class="agent-pending-zone"><b id="queue"></b></div>
        <div class="agent-composer-strip"><b id="strip"></b></div>
        <div class="agent-composer-region"><textarea id="composer"></textarea></div>
      </div>
      <div class="agent-messages-panel"><b id="messages"></b></div>
    </div>
  </div>`;

const at = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;

afterEach(() => {
    document.body.innerHTML = "";
});

describe("what an agent pane draws at the chrome's zoom", () => {
    it("is everything after the conversation, and Ctrl+wheel over it zooms chrome", () => {
        document.body.innerHTML = PANE;
        for (const id of ["desktop-bar", "request", "queue", "strip", "composer", "band"]) {
            expect(isOverChrome(at(id)), id).toBe(true);
        }
    });

    it("is not the conversation, the sign-in panel, the Messages panel or what lies over the conversation", () => {
        document.body.innerHTML = PANE;
        for (const id of ["conversation", "sign-in", "sign-in-notice", "messages", "btw"]) {
            expect(isOverChrome(at(id)), id).toBe(false);
        }
    });

    it("marks nothing in a pane whose transcript is absent (the picker and the history tab)", () => {
        document.body.innerHTML = `<div class="agent-view"><div class="agent-composer-region"><i id="x"></i></div></div>`;
        expect(isOverChrome(at("x"))).toBe(false);
    });
});

describe("the stylesheet", () => {
    const view = read("./agent-view.scss");

    it("applies the chrome zoom to the same selector the wheel uses, with the shared mixin and the pane's published level", () => {
        const rule = view.indexOf(`${AGENT_PANE_CHROME_SELECTOR} {`);
        expect(rule, "the rule for the siblings after the conversation").toBeGreaterThan(-1);
        const body = view.slice(rule, view.indexOf("}", rule));
        expect(body).toContain("@include mixins.chrome-zoom-inside(--agent-pane-zoom);");
    });

    it("keeps the controls as direct children: no wrapper class is added around them", () => {
        expect(AGENT_PANE_CHROME_SELECTOR.startsWith(".agent-view-zoomed > ")).toBe(true);
    });

    it("draws the top band at the chrome's zoom, not the pane's", () => {
        const band = read("./styles/_top-band.scss");
        expect(band).toMatch(/\.agent-top-band \{[^}]*zoom: var\(--zoomfactor\);/);
        expect(read("./top-band/AgentTopBand.tsx")).not.toMatch(/zoom:\s*props/);
    });

    it("sizes the shell drawer's edge handle from the composer's level (the chrome's), not the pane's", () => {
        const strip = read("./styles/_composer-strip.scss");
        const handle = strip.slice(strip.indexOf(".agent-composer-details-resize-handle--edge {"));
        const end = handle.indexOf(".agent-composer-details-body");
        expect(handle.slice(0, end)).toContain("var(--zoomfactor)");
        expect(handle.slice(0, end)).not.toContain("--agent-pane-zoom");
    });
});
