// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { connectionName, defaultAlias, parseDestination } from "./add-remote";

describe("parseDestination", () => {
    it("reads host, user@host, and a port", () => {
        expect(parseDestination("db1")).toEqual({ user: "", hostname: "db1", port: "" });
        expect(parseDestination(" me@db1.example.com ")).toEqual({ user: "me", hostname: "db1.example.com", port: "" });
        expect(parseDestination("asafe@127.0.0.1:2222")).toEqual({ user: "asafe", hostname: "127.0.0.1", port: "2222" });
    });

    it("reads IPv6, bare or in brackets with a port", () => {
        expect(parseDestination("fe80::1")).toEqual({ user: "", hostname: "fe80::1", port: "" });
        expect(parseDestination("me@[fe80::1]:2222")).toEqual({ user: "me", hostname: "fe80::1", port: "2222" });
    });

    it("is null for what isn't a destination", () => {
        for (const bad of ["", "  ", "@host", "me@", "me@host:port", "host:99999", "two words"]) {
            expect(parseDestination(bad), bad).toBeNull();
        }
    });
});

describe("defaultAlias", () => {
    it("is a host name's first label, or an address made safe with its port", () => {
        expect(defaultAlias({ user: "", hostname: "db1.example.com", port: "" })).toBe("db1");
        expect(defaultAlias({ user: "me", hostname: "db1.example.com", port: "2222" })).toBe("db1");
        expect(defaultAlias({ user: "asafe", hostname: "127.0.0.1", port: "2222" })).toBe("127-0-0-1-2222");
        expect(defaultAlias({ user: "", hostname: "10.0.0.5", port: "" })).toBe("10-0-0-5");
        expect(defaultAlias({ user: "", hostname: "fe80::1", port: "" })).toBe("fe80-1");
    });
});

describe("connectionName", () => {
    it("is user@host:port, with no port when none was typed (ssh config's, else 22), IPv6 in brackets with a port", () => {
        expect(connectionName({ user: "asafe", hostname: "127.0.0.1", port: "2222" })).toBe("asafe@127.0.0.1:2222");
        expect(connectionName({ user: "asafe", hostname: "127.0.0.1", port: "" })).toBe("asafe@127.0.0.1");
        expect(connectionName({ user: "", hostname: "db1", port: "" })).toBe("db1");
        expect(connectionName({ user: "me", hostname: "fe80::1", port: "2222" })).toBe("me@[fe80::1]:2222");
        expect(connectionName({ user: "", hostname: "fe80::1", port: "" })).toBe("fe80::1");
    });
});
