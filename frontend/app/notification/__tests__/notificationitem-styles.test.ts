// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The toast card's styles must match the toast itself.
 *
 * Every rule used to be nested under `.notification { … }`, compiling to
 * `.notification .notification-bubble`, `.notification .notification-inner`, …
 * The toast renders as `.notification-bubble` with no `.notification` ancestor,
 * so none applied and it drew as bare white text over the status bar.
 * jsdom has no SCSS, so this reads the source.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const scss = readFileSync(join(__dirname, "..", "notificationitem.scss"), "utf8");

describe("notification item styles", () => {
    it("the toast card's own box is a top-level rule", () => {
        expect(scss).toMatch(/^\.notification-bubble \{/m);
        expect(scss).not.toMatch(/^ {4}\.notification-bubble \{/m);
    });

    it("the shared inner rules (title, message, icon, close) apply to the toast too", () => {
        expect(scss).toMatch(/^\.notification,\n\.notification-bubble \{/m);
    });
});
