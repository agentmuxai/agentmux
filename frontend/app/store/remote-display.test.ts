// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { remoteDisplay } from "./remote-display";

describe("remoteDisplay", () => {
    const connections = {
        db1: { "display:name": " prod-db ", "display:color": "#e5484d" },
        web: { "display:color": "red" },
    } as Record<string, ConnKeywords>;

    it("is nothing for this computer", () => {
        expect(remoteDisplay(connections, "")).toBeNull();
        expect(remoteDisplay(connections, null)).toBeNull();
        expect(remoteDisplay(connections, "local")).toBeNull();
    });

    it("gives the nickname and a valid colour", () => {
        expect(remoteDisplay(connections, "db1")).toEqual({ name: "prod-db", color: "#e5484d" });
        expect(remoteDisplay(connections, "web")).toEqual({ name: "web", color: undefined });
        expect(remoteDisplay(undefined, "box")).toEqual({ name: "box", color: undefined });
    });
});
