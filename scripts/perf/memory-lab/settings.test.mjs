// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { parseAbArgs, planRuns, runArgs } from "./lib/abplan.mjs";
import { parseSettingPairs, parseSettingValue, settingsFileText, settingsLocations, settingsSeen, writeSettings } from "./lib/settings.mjs";

describe("--setting values", () => {
    it("reads booleans, numbers, JSON and plain text", () => {
        expect(parseSettingValue("true")).toBe(true);
        expect(parseSettingValue("false")).toBe(false);
        expect(parseSettingValue("12")).toBe(12);
        expect(parseSettingValue("0.5")).toBe(0.5);
        expect(parseSettingValue('["a","b"]')).toEqual(["a", "b"]);
        expect(parseSettingValue('"quoted"')).toBe("quoted");
        expect(parseSettingValue("default-dark")).toBe("default-dark");
        expect(() => parseSettingValue("{not json")).toThrow(/starts like JSON/);
    });

    it("reads NAME=value pairs and refuses a malformed one", () => {
        expect(parseSettingPairs(["agent:transcriptsinglecopy=true", "term:fontsize=15"])).toEqual({ "agent:transcriptsinglecopy": true, "term:fontsize": 15 });
        expect(parseSettingPairs(undefined)).toEqual({});
        expect(() => parseSettingPairs(["novalue"])).toThrow(/NAME=value/);
        expect(() => parseSettingPairs(["=x"])).toThrow(/NAME=value/);
        expect(() => parseSettingPairs(["bad name=1"])).toThrow(/NAME=value/);
    });

    it("writes the same file for the same settings, whatever the order they were given in", () => {
        expect(settingsFileText({ b: 1, a: true })).toBe(settingsFileText({ a: true, b: 1 }));
        expect(JSON.parse(settingsFileText({ "agent:transcriptsinglecopy": true }))).toEqual({ "agent:transcriptsinglecopy": true });
    });
});

describe("where the settings file goes", () => {
    let home;
    afterEach(() => rmSync(home, { recursive: true, force: true }));

    it("is the channels folder and every channel's config folder that exists", () => {
        home = mkdtempSync(join(tmpdir(), "lab-settings-"));
        mkdirSync(join(home, "channels", "stable", "config"), { recursive: true });
        mkdirSync(join(home, "channels", "local-main-1", "config"), { recursive: true });
        mkdirSync(join(home, "channels", "no-config-here"), { recursive: true });
        const where = settingsLocations(home);
        expect(where).toContain(join(home, "channels"));
        expect(where).toContain(join(home, "channels", "stable", "config"));
        expect(where).toContain(join(home, "channels", "local-main-1", "config"));
        expect(where).not.toContain(join(home, "channels", "no-config-here", "config"));
        const written = writeSettings(home, { "agent:transcriptsinglecopy": true });
        expect(written).toHaveLength(3);
        for (const file of written) expect(JSON.parse(readFileSync(file, "utf8"))).toEqual({ "agent:transcriptsinglecopy": true });
    });
});

describe("what the app reports", () => {
    it("says per setting whether its full config shows the requested value, another one, or nothing", () => {
        const seen = settingsSeen({ a: true, b: 3, c: "x" }, { settings: { a: true, b: 4 } });
        expect(seen).toEqual({ a: true, b: false, c: null });
        expect(settingsSeen({ a: true }, null)).toEqual({ a: null });
    });
});

describe("--setting in an A/B", () => {
    it("gives each side its own settings, and both the ones after the lone --", () => {
        const spec = parseAbArgs(["--portable", "P", "--runs", "1", "--b-setting", "agent:transcriptsinglecopy=true", "--", "--setting", "term:fontsize=15"]);
        const [a1, b1] = planRuns(spec);
        expect(runArgs(a1, spec)).toEqual(["--portable", "P", "--label", "a1", "--out", "ab-out", "--setting", "term:fontsize=15"]);
        expect(runArgs(b1, spec)).toEqual(["--portable", "P", "--label", "b1", "--out", "ab-out", "--setting", "agent:transcriptsinglecopy=true", "--setting", "term:fontsize=15"]);
    });
});
