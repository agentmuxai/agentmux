// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { errorPageFailedUrl } from "./error-page-url";

const b64 = (s: string) => btoa(String.fromCharCode(...new TextEncoder().encode(s)));
const hex = (s: string) => [...new TextEncoder().encode(s)].map((b) => b.toString(16).padStart(2, "0")).join("");
const page = (code: string) => `<!DOCTYPE html><html><body><p>Could not load <code>${code}</code></p></body></html>`;

describe("errorPageFailedUrl", () => {
    it("reads the failed address from the host's marker", () => {
        const failed = "https://example.com/a b?q=can't&x=ü";
        const uri = `data:text/html;base64,${b64(page("ignored"))}#amux-failed-url=${hex(failed)}`;
        expect(errorPageFailedUrl(uri)).toBe(failed);
    });

    it("reads it from the page itself for an error page saved before the marker", () => {
        const uri = `data:text/html;base64,${b64(page("http://localhost:3000/?a=1&amp;b=&#39;x&#39;"))}`;
        expect(errorPageFailedUrl(uri)).toBe("http://localhost:3000/?a=1&b='x'");
    });

    it("leaves every other URL alone", () => {
        expect(errorPageFailedUrl("https://example.com/")).toBeUndefined();
        expect(errorPageFailedUrl("localhost:3000")).toBeUndefined();
        expect(errorPageFailedUrl(`data:text/html;base64,${b64("<h1>hi</h1>")}`)).toBeUndefined();
        expect(errorPageFailedUrl("data:text/html;base64,%%%not-base64")).toBeUndefined();
        expect(errorPageFailedUrl("data:text/html;base64,PGgxPg==#amux-failed-url=zz")).toBeUndefined();
    });
});
