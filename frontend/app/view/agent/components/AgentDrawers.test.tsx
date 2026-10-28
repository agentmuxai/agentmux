// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DOM placement of the shell drawer, which is load-bearing
 * (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.3): the shell must render
 * as a flex child of `.agent-view`, not wrapped. Moving the JSX into
 * components must not add a wrapper element.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./AgentShellInfoPanel", () => ({ AgentShellInfoPanel: () => <div class="stub-shell-info" /> }));
vi.mock("./AgentShellSubblock", () => ({ AgentShellSubblock: () => <div class="stub-shell-subblock" /> }));

import { AgentShellDrawer } from "./AgentShellDrawer";

afterEach(cleanup);

const shell = (open: boolean) => (
    <div class="agent-view">
        <AgentShellDrawer
            open={open}
            blockId="b1"
            shellSubBlockId={undefined}
            cwd="/w"
            persistedHeight={undefined}
            onTermReady={() => {}}
            onTermDispose={() => {}}
            onShellExited={() => {}}
        />
    </div>
);

describe("AgentShellDrawer", () => {
    it("renders nothing while closed", () => {
        const { container } = render(() => shell(false));
        expect(container.querySelector(".agent-view")!.children).toHaveLength(0);
    });

    it("renders .agent-composer-details as a direct child of .agent-view", () => {
        const { container } = render(() => shell(true));
        const view = container.querySelector(".agent-view")!;
        expect(view.children).toHaveLength(1);
        const details = view.firstElementChild!;
        expect(details.className).toBe("agent-composer-details");
        expect(details.id).toBe("agent-composer-details-b1");
        expect(details.querySelector(".stub-shell-info")).not.toBeNull();
        expect(details.querySelector(".stub-shell-subblock")).not.toBeNull();
    });
});
