// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { PROVIDERS } from "./catalog";
import { installPackageField } from "./install-request";

describe("installPackageField", () => {
    it("is the npm package for an npm CLI", () => {
        expect(installPackageField({ id: "claude", npmPackage: "@anthropic-ai/claude-code" })).toBe("@anthropic-ai/claude-code");
    });

    it("asks for an install by id for a CLI AgentMux downloads", () => {
        expect(installPackageField(PROVIDERS.goose)).toBe("goose");
    });

    it("stays empty for a CLI AgentMux does not install", () => {
        expect(installPackageField({ id: "kimi", npmPackage: "" })).toBe("");
    });
});
