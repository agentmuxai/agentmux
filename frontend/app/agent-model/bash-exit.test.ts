// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { bashExitCode, parseExitPrefix } from "./bash-exit";

describe("parseExitPrefix", () => {
    it("splits bashwrap's `<exited N in Ts>` line off stdout", () => {
        expect(parseExitPrefix("<exited 0 in 3.56s>\ntotal 8")).toEqual({ exit: 0, body: "total 8" });
        expect(parseExitPrefix("<exited -1 in 0.10s>")).toEqual({ exit: -1, body: "" });
    });

    it("leaves output without the prefix alone", () => {
        expect(parseExitPrefix("hello")).toEqual({ exit: undefined, body: "hello" });
        expect(parseExitPrefix(undefined)).toEqual({ exit: undefined, body: "" });
    });
});

describe("bashExitCode", () => {
    it("prefers a native exitCode", () => {
        expect(bashExitCode({ exitCode: 2, stdout: "<exited 0 in 1s>" })).toBe(2);
    });

    it("reads the prefix from stdout, or from the translator's {content} shape", () => {
        expect(bashExitCode({ stdout: "<exited 1 in 1.23s>\nboom" })).toBe(1);
        expect(bashExitCode({ content: "<exited 0 in 0.60s>\nok" })).toBe(0);
    });

    it("is undefined when neither carries one", () => {
        expect(bashExitCode({ stdout: "plain" })).toBeUndefined();
        expect(bashExitCode(null)).toBeUndefined();
    });
});
