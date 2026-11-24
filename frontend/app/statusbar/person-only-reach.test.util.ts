// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the UI tool's click and query reach on a rendered page: the host's own page helper
 * (crates/cef/src/browser_api/scripts/query.js) run against the DOM a test rendered, for an agent whose pane is
 * `blockId`. A control in a prompt marked `data-operator-only` is out of reach.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SOURCE = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../../../crates/cef/src/browser_api/scripts/query.js"), "utf8");

type Helpers = {
    __amq_allowed_for: (el: Element, blockId: string) => boolean;
    __amq_centroid_of: (selector: string, blockId: string) => unknown;
};

export function agentReach(blockId = "agent-pane") {
    const win = window as unknown as Partial<Helpers> & Record<string, unknown>;
    if (!win.__amq_allowed_for) {
        new Function("window", "document", "CSS", SOURCE)(window, document, { escape: (s: string) => s });
    }
    const h = window as unknown as Helpers;
    return {
        /** Whether the tool may read or press `el`. */
        allowed: (el: Element): boolean => h.__amq_allowed_for(el, blockId),
        /** The tool's click on the first button matching `selector`: `operatorOnly` when it is a prompt for the person. */
        click: (selector: string): unknown => {
            const target = [...document.querySelectorAll(selector)].find((e) => h.__amq_allowed_for(e, blockId));
            (Element.prototype as unknown as { scrollIntoView: () => void }).scrollIntoView = () => {};
            (document as unknown as { elementFromPoint: () => Element | null }).elementFromPoint = () => target ?? null;
            return h.__amq_centroid_of(selector, blockId);
        },
    };
}
