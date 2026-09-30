// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DOM placement of the stash and shell drawers, which is load-bearing
 * (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.3): the stash drawer's 50%
 * height cap only resolves when its resizable element is a DIRECT child of
 * `.agent-view` (reagentx P1 on PR #3540), and the shell must render as a flex
 * child of `.agent-view`, not wrapped. Moving the JSX into components must not
 * add a wrapper element.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./AgentStashModal", () => ({ AgentStashModal: () => <div class="stub-stash-modal" /> }));
vi.mock("./AgentShellInfoPanel", () => ({ AgentShellInfoPanel: () => <div class="stub-shell-info" /> }));
vi.mock("./AgentShellSubblock", () => ({ AgentShellSubblock: () => <div class="stub-shell-subblock" /> }));

import { AgentShellDrawer } from "./AgentShellDrawer";
import { AgentStashDrawer } from "./AgentStashDrawer";

afterEach(cleanup);

const stash = (open: boolean) => (
    <div class="agent-view">
        <AgentStashDrawer
            open={open}
            blockId="b1"
            persistedHeight={undefined}
            agentId="a1"
            agentName="Ada"
            workingDirectory="/w"
            hasDefinition={true}
        />
    </div>
);

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

describe("AgentStashDrawer", () => {
    it("renders nothing while closed", () => {
        const { container } = render(() => stash(false));
        expect(container.querySelector(".agent-view")!.children).toHaveLength(0);
    });

    it("renders its resizable drawer as a direct child of .agent-view", () => {
        const { container } = render(() => stash(true));
        const view = container.querySelector(".agent-view")!;
        expect(view.children).toHaveLength(1);
        expect(view.firstElementChild!.className).toContain("agent-stash-drawer");
        expect(view.querySelector(".stub-stash-modal")).not.toBeNull();
    });
});

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
