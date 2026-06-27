// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the per-block error boundary (cascade follow-up 1 of 4).
 *
 * Contract:
 *   - A renderer throw inside one BlockErrorBoundary renders the localized
 *     fallback ("This pane crashed" + message + reset button).
 *   - The throw is forwarded to the host via `fe_log_structured`.
 *   - Two sibling BlockErrorBoundary instances are independent: a throw in
 *     one does NOT blank the sibling.
 *   - The "Reload pane" button calls the SolidJS ErrorBoundary reset
 *     callback, which re-mounts the children.
 *
 * Spec: docs/retro/retro-agent-pane-cascade-replacechild-2026-05-23.md.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Hoisted because vi.mock factories run before module imports.
const invokeCommandMock = vi.fn<(cmd: string, args: Record<string, unknown>) => Promise<void>>(() => Promise.resolve());
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: invokeCommandMock,
}));

// Suppress SolidJS's own console.error of the caught exception so the
// vitest log isn't drowned in red. The boundary still catches and the
// fallback still renders; we just don't want the noise.
let consoleErrorSpy: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
    invokeCommandMock.mockClear();
    consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
    cleanup();
    consoleErrorSpy.mockRestore();
});

// Component that throws on first render. Used as the child of the boundary.
function Boom(props: { message?: string }): never {
    throw new Error(props.message ?? "boom");
}

function Healthy(props: { label: string }) {
    return <div data-testid={`healthy-${props.label}`}>{props.label}</div>;
}

// Lazy import after the mock is registered.
async function loadBoundary() {
    const mod = await import("./BlockErrorBoundary");
    return mod.BlockErrorBoundary;
}

describe("BlockErrorBoundary — single-pane fallback", () => {
    it("renders the fallback when the child throws on mount", async () => {
        const BlockErrorBoundary = await loadBoundary();
        render(() => (
            <BlockErrorBoundary blockId="abcdef1234" viewType="agent">
                <Boom message="kaboom" />
            </BlockErrorBoundary>
        ));

        expect(screen.getByTestId("block-error-fallback")).toBeInTheDocument();
        expect(screen.getByText(/This pane crashed/)).toBeInTheDocument();
        // The thrown message text appears in the body.
        expect(screen.getByText(/kaboom/)).toBeInTheDocument();
        // Reload button is the primary affordance.
        expect(
            screen.getByTestId("block-error-fallback-reload"),
        ).toBeInTheDocument();
    });

    it("shows the short block id and view type in the fallback meta line", async () => {
        const BlockErrorBoundary = await loadBoundary();
        render(() => (
            <BlockErrorBoundary blockId="abcdef1234567890" viewType="browser">
                <Boom />
            </BlockErrorBoundary>
        ));

        // 7-char short id.
        const fallback = screen.getByTestId("block-error-fallback");
        expect(fallback.textContent).toContain("abcdef1");
        expect(fallback.textContent).toContain("browser");
    });

    it("renders the Close pane button only when onClose is provided", async () => {
        const BlockErrorBoundary = await loadBoundary();
        const onClose = vi.fn();
        const { unmount } = render(() => (
            <BlockErrorBoundary blockId="aaa" viewType="agent" onClose={onClose}>
                <Boom />
            </BlockErrorBoundary>
        ));

        const closeBtn = screen.getByTestId("block-error-fallback-close");
        expect(closeBtn).toBeInTheDocument();
        const user = userEvent.setup();
        await user.click(closeBtn);
        expect(onClose).toHaveBeenCalledTimes(1);
        unmount();
        cleanup();

        // No onClose: no Close button.
        render(() => (
            <BlockErrorBoundary blockId="aaa" viewType="agent">
                <Boom />
            </BlockErrorBoundary>
        ));
        expect(
            screen.queryByTestId("block-error-fallback-close"),
        ).toBeNull();
    });

    it("toggles the stack section when 'Show stack' is clicked", async () => {
        const BlockErrorBoundary = await loadBoundary();
        render(() => (
            <BlockErrorBoundary blockId="aaa" viewType="agent">
                <Boom message="stackable" />
            </BlockErrorBoundary>
        ));

        // Initially the stack <pre> isn't in the document.
        expect(screen.queryByText(/at Boom/)).toBeNull();
        const toggle = screen.getByRole("button", { name: /Show stack/ });
        const user = userEvent.setup();
        await user.click(toggle);
        // After click, the button text flips; the <pre> renders.
        expect(
            screen.getByRole("button", { name: /Hide stack/ }),
        ).toBeInTheDocument();
    });
});

describe("BlockErrorBoundary — logging side effect", () => {
    it("forwards the catch via fe_log_structured with block_id + view_type + stack", async () => {
        const BlockErrorBoundary = await loadBoundary();
        render(() => (
            <BlockErrorBoundary blockId="0123456789abcdef" viewType="agent">
                <Boom message="logged" />
            </BlockErrorBoundary>
        ));

        // SolidJS ErrorBoundary catches synchronously during render, so by
        // the time the fallback is in the DOM the invokeCommand mock has
        // already been called.
        expect(invokeCommandMock).toHaveBeenCalledTimes(1);
        const [cmd, args] = invokeCommandMock.mock.calls[0]!;
        expect(cmd).toBe("fe_log_structured");
        expect(args).toMatchObject({
            level: "error",
            module: "block-error-boundary",
        });
        expect(args.data).toMatchObject({
            block_id: "0123456789abcdef",
            view_type: "agent",
            error_name: "Error",
            error_message: "logged",
        });
        expect(typeof (args.data as Record<string, unknown>).error_stack === "string").toBe(true);
        expect(args.message).toMatch(/block-error-boundary/);
    });

    it("survives an invokeCommand throw (logging must not break the fallback)", async () => {
        invokeCommandMock.mockImplementationOnce(() => {
            throw new Error("ipc-dead");
        });
        const BlockErrorBoundary = await loadBoundary();
        render(() => (
            <BlockErrorBoundary blockId="x" viewType="agent">
                <Boom message="survive" />
            </BlockErrorBoundary>
        ));
        // Fallback still rendered.
        expect(screen.getByTestId("block-error-fallback")).toBeInTheDocument();
        expect(screen.getByText(/survive/)).toBeInTheDocument();
    });
});

describe("BlockErrorBoundary — sibling isolation", () => {
    it("a throw in one boundary leaves a sibling boundary's children mounted", async () => {
        const BlockErrorBoundary = await loadBoundary();
        render(() => (
            <div>
                <BlockErrorBoundary blockId="aaaaaaa" viewType="agent">
                    <Boom message="left-pane-crashed" />
                </BlockErrorBoundary>
                <BlockErrorBoundary blockId="bbbbbbb" viewType="term">
                    <Healthy label="right" />
                </BlockErrorBoundary>
            </div>
        ));

        // The throwing pane's fallback is shown.
        expect(screen.getByTestId("block-error-fallback")).toBeInTheDocument();
        expect(screen.getByText(/left-pane-crashed/)).toBeInTheDocument();
        // The healthy sibling stayed mounted.
        expect(screen.getByTestId("healthy-right")).toBeInTheDocument();
        // Only one fallback in the document — the sibling did NOT render
        // its boundary's fallback.
        expect(screen.queryAllByTestId("block-error-fallback")).toHaveLength(1);
    });
});

describe("BlockErrorBoundary — reset / reload pane", () => {
    it("clicking 'Reload pane' re-mounts the children and clears the fallback", async () => {
        const BlockErrorBoundary = await loadBoundary();
        // First mount throws; after reset, swap to a healthy child.
        let shouldThrow = true;
        const Conditional = () => {
            if (shouldThrow) {
                throw new Error("first-mount-throw");
            }
            return <Healthy label="recovered" />;
        };

        render(() => (
            <BlockErrorBoundary blockId="resetid" viewType="agent">
                <Conditional />
            </BlockErrorBoundary>
        ));

        expect(screen.getByTestId("block-error-fallback")).toBeInTheDocument();

        // Flip the gate then click Reload — the boundary's reset re-runs
        // the children, this time without throwing.
        shouldThrow = false;
        const user = userEvent.setup();
        await user.click(screen.getByTestId("block-error-fallback-reload"));

        // Fallback gone, healthy child mounted.
        expect(screen.queryByTestId("block-error-fallback")).toBeNull();
        expect(screen.getByTestId("healthy-recovered")).toBeInTheDocument();
    });
});
