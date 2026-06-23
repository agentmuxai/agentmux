// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    extractFetchResult,
    httpStatusText,
    looksLikeJson,
    statusClass,
} from "./web-fetch-result";

describe("extractFetchResult", () => {
    it("accepts a plain string", () => {
        const out = extractFetchResult("hello world");
        expect(out).toEqual({ content: "hello world" });
    });

    it("accepts a structured object with content", () => {
        const out = extractFetchResult({
            url: "https://example.com/page",
            title: "Example Page",
            status: 200,
            content: "Page body text",
            truncated: false,
            contentType: "text/html",
        });
        expect(out).toEqual({
            url: "https://example.com/page",
            title: "Example Page",
            status: 200,
            content: "Page body text",
            truncated: false,
            contentType: "text/html",
        });
    });

    it("accepts alternate field names (body, uri, status_code, content_type)", () => {
        const out = extractFetchResult({
            uri: "https://example.com",
            body: "body text",
            status_code: 404,
            content_type: "text/plain",
        });
        expect(out).toMatchObject({
            url: "https://example.com",
            content: "body text",
            status: 404,
            contentType: "text/plain",
        });
    });

    it("accepts truncated flag via is_truncated", () => {
        const out = extractFetchResult({ content: "...", is_truncated: true });
        expect(out?.truncated).toBe(true);
    });

    it("returns null for empty string", () => {
        expect(extractFetchResult("")).toBeNull();
        expect(extractFetchResult("   ")).toBeNull();
    });

    it("returns null for object missing content", () => {
        expect(extractFetchResult({ url: "https://x.com", status: 200 })).toBeNull();
    });

    it("returns null for null, undefined, array, number", () => {
        expect(extractFetchResult(null)).toBeNull();
        expect(extractFetchResult(undefined)).toBeNull();
        expect(extractFetchResult([])).toBeNull();
        expect(extractFetchResult(42)).toBeNull();
    });

    it("omits undefined optional fields", () => {
        const out = extractFetchResult({ content: "hi" });
        expect(out?.url).toBeUndefined();
        expect(out?.title).toBeUndefined();
        expect(out?.status).toBeUndefined();
    });
});

describe("looksLikeJson", () => {
    it("detects JSON objects and arrays", () => {
        expect(looksLikeJson('{"key":"val"}')).toBe(true);
        expect(looksLikeJson('[{"url":"x"}]')).toBe(true);
    });

    it("returns false for plain text", () => {
        expect(looksLikeJson("plain text")).toBe(false);
        expect(looksLikeJson("<html>")).toBe(false);
    });
});

describe("httpStatusText", () => {
    it("returns known labels", () => {
        expect(httpStatusText(200)).toBe("OK");
        expect(httpStatusText(404)).toBe("Not Found");
        expect(httpStatusText(500)).toBe("Server Error");
    });

    it("returns raw string for unknown codes", () => {
        expect(httpStatusText(418)).toBe("418");
    });
});

describe("statusClass", () => {
    it("classifies status ranges", () => {
        expect(statusClass(200)).toBe("ok");
        expect(statusClass(302)).toBe("redirect");
        expect(statusClass(404)).toBe("error");
        expect(statusClass(503)).toBe("error");
    });
});
