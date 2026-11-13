// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { makeTestHostApi } from "@/app/host/test-host";
import { AUTH_KEY_HEADER } from "@/util/sharedconst";
import { authHeaders } from "./auth-headers";

describe("authHeaders", () => {
    afterEach(() => {
        window.api = undefined as unknown as AppApi;
    });

    it("sends the key the host gave, over any extra headers", () => {
        window.api = makeTestHostApi({ getAuthKey: () => "k1" });
        expect(authHeaders()).toEqual({ "X-AuthKey": "k1" });
        expect(authHeaders({ "Content-Type": "application/json" })).toEqual({
            "Content-Type": "application/json",
            "X-AuthKey": "k1",
        });
    });

    it("sends the tab's session when the host holds no key (a browser signed in by srv)", () => {
        window.api = makeTestHostApi({ getAuthKey: () => "", getSessionToken: () => "tok" });
        expect(authHeaders({ Range: "bytes=0-1" })).toEqual({ Range: "bytes=0-1", "X-AM-Session": "tok" });
    });

    it("sends the key alone when a host has both, and no session header when there is none", () => {
        window.api = makeTestHostApi({ getAuthKey: () => "k1", getSessionToken: () => "tok" });
        expect(authHeaders()).toEqual({ "X-AuthKey": "k1" });
        window.api = makeTestHostApi({ getAuthKey: () => "", getSessionToken: () => undefined });
        expect(authHeaders()).toEqual({});
    });

    it("sends no X-AuthKey when the host gives none (a proxy adds it)", () => {
        window.api = makeTestHostApi({ getAuthKey: () => "" });
        expect(authHeaders({ Range: "bytes=0-1" })).toEqual({ Range: "bytes=0-1" });
    });

    it("sends no X-AuthKey before there is a host", () => {
        expect(authHeaders()).toEqual({});
    });
});

describe("AUTH_KEY_HEADER", () => {
    // The literal is spelled out in the tests on purpose: they pin the wire
    // name, so changing the constant alone fails here rather than passing.
    it("is the name srv's Rust clients send", () => {
        const rust = readFileSync(resolve(__dirname, "../../../crates/common/src/lib.rs"), "utf8");
        const m = rust.match(/pub const AUTH_KEY_HEADER: &str = "([^"]+)";/);
        expect(m?.[1]).toBe(AUTH_KEY_HEADER);
        expect(AUTH_KEY_HEADER).toBe("X-AuthKey");
    });
});
