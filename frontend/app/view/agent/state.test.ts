// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// NOTE: These tests are temporarily disabled while migrating from Jotai to SolidJS signals.
// TODO: Rewrite tests using SolidJS testing utilities.

import { describe, test, expect, beforeEach } from "vitest";
import {
    createAgentAtoms,
    type AgentAtoms,
} from "./state";
import type { DocumentNode, MarkdownNode, ToolNode } from "./types";

let atoms: AgentAtoms;

beforeEach(() => {
    atoms = createAgentAtoms("test-block-1");
});

describe("createAgentAtoms", () => {
    test("creates signals with correct default values", () => {
        const [getDoc] = atoms.documentAtom;
        const [getRaw] = atoms.rawOutputAtom;
        const [getSession] = atoms.sessionIdAtom;
        const [getAuth] = atoms.authAtom;
        const [getUserInfo] = atoms.userInfoAtom;
        const [getProviderConfig] = atoms.providerConfigAtom;

        expect(getDoc()).toEqual([]);
        expect(getRaw()).toBe("");
        expect(getSession()).toBe("");
        expect(getAuth()).toEqual({ status: "disconnected" });
        expect(getUserInfo()).toBeNull();
        expect(getProviderConfig()).toBeNull();
    });

    test("rawOutputAtom can be updated", () => {
        const [getRaw, setRaw] = atoms.rawOutputAtom;
        setRaw("hello world");
        expect(getRaw()).toBe("hello world");
    });

    test("rawOutputAtom can accumulate output", () => {
        const [getRaw, setRaw] = atoms.rawOutputAtom;
        setRaw("line 1\n");
        setRaw(getRaw() + "line 2\n");
        expect(getRaw()).toBe("line 1\nline 2\n");
    });

    test("authAtom defaults to disconnected", () => {
        const [getAuth] = atoms.authAtom;
        expect(getAuth().status).toBe("disconnected");
    });

    test("authAtom can be set to connected", () => {
        const [getAuth, setAuth] = atoms.authAtom;
        setAuth({ status: "connected" });
        expect(getAuth().status).toBe("connected");
    });

    test("documentStateAtom has correct default filter", () => {
        const [getState] = atoms.documentStateAtom;
        const state = getState();
        expect(state.filter.showThinking).toBe(false);
        expect(state.filter.showSuccessfulTools).toBe(true);
        expect(state.filter.showFailedTools).toBe(true);
        expect(state.filter.showIncoming).toBe(true);
        expect(state.filter.showOutgoing).toBe(true);
    });

    test("separate instances have independent state", () => {
        const atoms2 = createAgentAtoms("test-block-2");
        const [, setRaw1] = atoms.rawOutputAtom;
        const [getRaw1] = atoms.rawOutputAtom;
        const [, setRaw2] = atoms2.rawOutputAtom;
        const [getRaw2] = atoms2.rawOutputAtom;

        setRaw1("instance 1");
        setRaw2("instance 2");
        expect(getRaw1()).toBe("instance 1");
        expect(getRaw2()).toBe("instance 2");
    });
});
