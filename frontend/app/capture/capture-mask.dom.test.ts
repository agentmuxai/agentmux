// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { openListRefusal, PRIVATE_SELECTOR } from "./capture-mask";
import { overlayAnchor, setOverlayAnchor } from "./overlay-anchors";

afterEach(() => (document.body.innerHTML = ""));

describe("a list of Vault entries drawn by the system", () => {
    it("refuses the capture while the select inside a private panel has focus", () => {
        document.body.innerHTML = `<div class="setting-vault-key"><select><option>a</option></select></div><select id="other"><option>b</option></select>`;
        const inPanel = document.querySelector(".setting-vault-key select") as HTMLSelectElement;
        inPanel.focus();
        expect(openListRefusal(document.activeElement)).toBe("a list of Vault entries may be open");
        (document.getElementById("other") as HTMLSelectElement).focus();
        expect(openListRefusal(document.activeElement)).toBeUndefined();
        expect(openListRefusal(null)).toBeUndefined();
    });

    it("covers the Jobs editor's entry picker and the Settings key field", () => {
        for (const cls of ["job-vault-entry", "setting-vault-key", "job-vault-secrets"]) {
            document.body.innerHTML = `<div class="${cls}"></div>`;
            expect(document.querySelectorAll(PRIVATE_SELECTOR)).toHaveLength(1);
        }
    });
});

describe("an overlay's anchor", () => {
    it("is the element it was opened for", () => {
        document.body.innerHTML = `<div data-blockid="v1"><button id="b">x</button></div><div id="tip" data-pane-overlay></div>`;
        const tip = document.getElementById("tip")!;
        setOverlayAnchor(tip, document.getElementById("b")!);
        expect(overlayAnchor(tip)?.closest("[data-blockid]")?.getAttribute("data-blockid")).toBe("v1");
    });
});
