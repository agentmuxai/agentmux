// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * In a small pane the new-agent panel opens inside a list that scrolls, and its install steps sit below the fold:
 * a person who clicked Install now saw the list jump and never the run. The panel brings the steps into view when
 * the run starts or fails, and itself when one step replaces another (the install ends, Create follows).
 */

// @vitest-environment jsdom

import { cleanup, render } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./create-panel-model", () => ({ open: vi.fn(), STEP_LABEL: { tools: "Tools", install: "Install", create: "Create" } }));
vi.mock("@/element/install/InstallProgress", () => ({ InstallProgress: () => <div data-testid="progress" /> }));
vi.mock("./AgentCreateForm", () => ({ AgentCreateForm: () => <div data-testid="form" /> }));
vi.mock("./FreeStartGuide", () => ({ FreeStartGuide: () => <div /> }));
vi.mock("./create-panel-tools-view", () => ({ ToolsStep: () => <div /> }));

import type { InstallState } from "@/element/install/install-session";
import { NpmInstall } from "./create-panel-install-view";
import { PanelView, type PanelParts } from "./create-panel-view";

const scroll = vi.fn();

beforeEach(() => {
    scroll.mockClear();
    Element.prototype.scrollIntoView = scroll as never;
});
afterEach(cleanup);

function fakes() {
    return createRoot(() => build());
}

function build() {
    const [state, setState] = createSignal<InstallState>("idle");
    const [step, setStep] = createSignal<"install" | "create">("install");
    const session = { state, start: vi.fn(), cancel: vi.fn() };
    const p = {
        s: {
            name: () => "Claude Code",
            userInstalled: () => false,
            props: { template: { name: "Claude Code" }, onClose: vi.fn(), recheckInstalled: vi.fn() },
            runtime: { pickRuntime: vi.fn() },
        },
        free: { showFreeGuide: () => false },
        install: { session: () => session, setInstallDone: vi.fn() },
        flow: { steps: () => ["install", "create"], step, offerContainer: () => false },
    } as unknown as PanelParts;
    return { p, session, setState, setStep };
}

describe("the install panel in a small pane", () => {
    it("does not move the list when it opens, idle", () => {
        const { p, session } = fakes();
        render(() => <NpmInstall p={p} session={() => session as never} />);
        expect(scroll).not.toHaveBeenCalled();
    });

    it("brings its steps into view when the run starts, and again if it fails", () => {
        const { p, session, setState } = fakes();
        const { getByTestId } = render(() => <NpmInstall p={p} session={() => session as never} />);
        setState("running");
        expect(scroll).toHaveBeenCalledTimes(1);
        expect(scroll.mock.contexts[0]).toBe(getByTestId("agent-create-npm-install"));
        expect(scroll).toHaveBeenLastCalledWith({ block: "nearest", inline: "nearest" });
        setState("failed");
        expect(scroll).toHaveBeenCalledTimes(2);
    });

    it("does not scroll again for the end of a successful run: the next step keeps itself in view", () => {
        const { p, session, setState } = fakes();
        render(() => <NpmInstall p={p} session={() => session as never} />);
        setState("running");
        scroll.mockClear();
        setState("done");
        expect(scroll).not.toHaveBeenCalled();
    });

    it("keeps the panel in view when one step replaces another, and not before", () => {
        const { p, setStep } = fakes();
        const { getByTestId } = render(() => <PanelView p={p} />);
        expect(scroll).not.toHaveBeenCalled();
        setStep("create");
        expect(scroll).toHaveBeenCalledTimes(1);
        expect(scroll.mock.contexts[0]).toBe(getByTestId("agent-create-panel"));
    });

    it("runs where there is no scrollIntoView (an older browser or a test DOM)", () => {
        delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
        const { p, session, setState } = fakes();
        render(() => <NpmInstall p={p} session={() => session as never} />);
        expect(() => setState("running")).not.toThrow();
    });
});
