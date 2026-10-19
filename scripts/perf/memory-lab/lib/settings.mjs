// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// App settings the lab gives the instance it starts (`--setting NAME=value`). An experiment switch that is a setting
// (e.g. `agent:transcriptsinglecopy`) has to be in the instance's settings.json before srv reads it; an environment
// variable does not get through the launcher. The settings folder depends on the build's channel (a local build
// has its own, a release uses the shared one), which is only known once the instance has run, so the lab starts the
// instance once, writes the file where that build looks for it, and starts it again on the same data folder.

import { existsSync, mkdirSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** A setting's value from the command line: true/false, a number, a JSON array/object/string, else the text itself. */
export function parseSettingValue(text) {
    const t = String(text);
    if (t === "true") return true;
    if (t === "false") return false;
    if (/^-?\d+(\.\d+)?$/.test(t)) return Number(t);
    if (/^[[{"]/.test(t)) {
        try {
            return JSON.parse(t);
        } catch {
            throw new Error(`--setting value ${t} starts like JSON but isn't`);
        }
    }
    return t;
}

/** `NAME=value` pairs from repeated `--setting`, as an object; throws on a malformed pair. Names look like `agent:transcriptsinglecopy`. */
export function parseSettingPairs(pairs) {
    const out = {};
    for (const pair of pairs ?? []) {
        const eq = String(pair).indexOf("=");
        const name = eq > 0 ? pair.slice(0, eq) : "";
        if (!/^[A-Za-z][A-Za-z0-9_.:-]*$/.test(name)) throw new Error(`--setting needs NAME=value (got "${pair}")`);
        out[name] = parseSettingValue(pair.slice(eq + 1));
    }
    return out;
}

/** The settings.json text for the requested settings: plain JSON (the app's settings file accepts it), keys sorted so two runs write the same file. */
export function settingsFileText(settings) {
    return JSON.stringify(Object.fromEntries(Object.entries(settings).sort(([a], [b]) => (a < b ? -1 : 1))), null, 4) + "\n";
}

/**
 * Where a build may read settings.json under the data folder `home`: every channel's `config` folder that exists, and the
 * `channels` folder itself, where the shared (non-isolated) settings of a release channel live. Writing a file in a place the
 * build does not read is harmless.
 */
export function settingsLocations(home) {
    const channels = join(home, "channels");
    const out = [channels];
    if (existsSync(channels)) {
        for (const name of readdirSync(channels)) {
            const config = join(channels, name, "config");
            if (statSync(join(channels, name)).isDirectory() && existsSync(config)) out.push(config);
        }
    }
    return out;
}

/** Writes the settings file into every place the build may read it from; returns the paths written. */
export function writeSettings(home, settings) {
    const text = settingsFileText(settings);
    const written = [];
    for (const dir of settingsLocations(home)) {
        mkdirSync(dir, { recursive: true });
        const file = join(dir, "settings.json");
        writeFileSync(file, text);
        written.push(file);
    }
    return written;
}

/** Which of the requested settings the running app reports with the requested value in its full config (`settings`): { name: true|false|null }, null when the config has no such key. */
export function settingsSeen(requested, fullConfig) {
    const live = fullConfig?.settings ?? {};
    return Object.fromEntries(Object.entries(requested).map(([k, v]) => [k, k in live ? JSON.stringify(live[k]) === JSON.stringify(v) : null]));
}
