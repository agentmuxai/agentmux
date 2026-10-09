// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { makeTestHostApi } from "@/app/host/test-host";
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

    it("sends no X-AuthKey when the host gives none (a proxy adds it)", () => {
        window.api = makeTestHostApi({ getAuthKey: () => "" });
        expect(authHeaders({ Range: "bytes=0-1" })).toEqual({ Range: "bytes=0-1" });
    });

    it("sends no X-AuthKey before there is a host", () => {
        expect(authHeaders()).toEqual({});
    });
});
