// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The worked example in the text a new widgets.json starts with must work once uncommented: its key has to be one the
// widget bar can pin (action-widgets-config.ts), or the button shows neither on the bar nor in More.

import { describe, expect, it } from "vitest";
import { keyOfPinId, pinIdOf } from "@/app/window/action-widgets-config";
import TEMPLATE from "./widgets-json-template.jsonc?raw";

/** The example with the comment marks taken off, as the person would leave it: the commented lines inside the object. */
function uncommentedExample(): Record<string, Record<string, unknown>> {
    const live = TEMPLATE.split("\n")
        .map((l) => l.replace(/^ {4}\/\/ ?/, "    "))
        .filter((l) => !l.trimStart().startsWith("//"))
        .join("\n")
        .replace(/,(\s*[}\]])/g, "$1");
    return JSON.parse(live);
}

describe("the example in a new widgets.json", () => {
    it("has an entry once uncommented", () => {
        const example = uncommentedExample();
        expect(Object.keys(example)).toHaveLength(1);
        expect(Object.values(example)[0].label).toBe("Docs");
    });

    it("has a key the widget bar can pin and list", () => {
        for (const key of Object.keys(uncommentedExample())) {
            expect(keyOfPinId(pinIdOf(key)), `${key} does not survive the bar's pin id`).toBe(key);
        }
    });
});
