// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane view's Suspense boundary (PaneViewBoundary.tsx): a pending read in the view leaves the view in the
 * document, in the same place and with its scroll position, and shows the fallback over it. The plain
 * `<Suspense>` it replaces is measured beside it, so the difference this file is about stays visible.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createResource, createSignal, Show, Suspense, type JSX } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { PaneViewBoundary } from "./PaneViewBoundary";

afterEach(cleanup);

/** A read that stays pending until `settle` is called. */
function pendingRead<T>(): { read: () => Promise<T>; settle: (value: T) => Promise<void> } {
    let done!: (value: T) => void;
    const promise = new Promise<T>((resolve) => (done = resolve));
    return {
        read: () => promise,
        settle: async (value) => {
            done(value);
            await promise;
            await Promise.resolve();
        },
    };
}

/** A part of a view that reads a resource when it mounts. */
function Late(props: { read: () => Promise<string> }): JSX.Element {
    const [text] = createResource(props.read);
    return <p data-testid="late">{text() ?? "nothing yet"}</p>;
}

let mounts = 0;
/** A view with a scroller, and a part that mounts when `open` turns true. */
function View(props: { open: () => boolean; read: () => Promise<string> }): JSX.Element {
    mounts++;
    return (
        <div data-testid="view">
            <div data-testid="scroller" style={{ overflow: "auto", height: "100px" }}>
                <div style={{ height: "1000px" }}>rows</div>
            </div>
            <Show when={props.open()}>
                <Late read={props.read} />
            </Show>
        </div>
    );
}

function mount(boundary: "held" | "plain") {
    mounts = 0;
    const [open, setOpen] = createSignal(false);
    const pending = pendingRead<string>();
    const fallback = <span data-testid="pending">pending</span>;
    const view = () => <View open={open} read={pending.read} />;
    const { container } = render(() => (
        <div data-testid="content">
            {boundary === "held" ? <PaneViewBoundary fallback={fallback}>{view()}</PaneViewBoundary> : <Suspense fallback={fallback}>{view()}</Suspense>}
        </div>
    ));
    const find = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);
    return { find, setOpen, pending };
}

describe("a pending read in a pane's view", () => {
    it("leaves the view in the document, where it was and scrolled as it was", async () => {
        const { find, setOpen, pending } = mount("held");
        const view = find("view")!;
        const scroller = find("scroller")!;
        const place = view.parentElement;
        scroller.scrollTop = 400;
        expect(find("pending")).toBeNull();

        setOpen(true);
        expect(find("pending")).not.toBeNull();
        expect(view.isConnected).toBe(true);
        expect(find("view")).toBe(view);
        expect(view.parentElement).toBe(place);
        expect(find("scroller")).toBe(scroller);
        expect(scroller.scrollTop).toBe(400);
        // The part that reads is drawn with what it has.
        expect(find("late")!.textContent).toBe("nothing yet");

        await pending.settle("read");
        expect(find("pending")).toBeNull();
        expect(find("late")!.textContent).toBe("read");
        expect(find("view")).toBe(view);
        expect(scroller.scrollTop).toBe(400);
        expect(mounts).toBe(1);
    });

    it("was taken out of the document by a plain Suspense, which this boundary replaces", () => {
        const { find, setOpen } = mount("plain");
        const view = find("view")!;
        setOpen(true);
        expect(find("pending")).not.toBeNull();
        expect(view.isConnected).toBe(false);
    });

    it("gives the view no box of its own: the host is the content box's child and the view the host's", () => {
        const { find } = mount("held");
        const host = find("view")!.parentElement!;
        expect(host.className).toBe("pane-view-host");
        expect(host.parentElement).toBe(find("content"));
    });

    it("shows the fallback beside the host, not in place of it", () => {
        const { find, setOpen } = mount("held");
        setOpen(true);
        const mark = find("pending")!.parentElement!;
        expect(mark.className).toBe("pane-view-pending");
        expect(mark.parentElement).toBe(find("content"));
        expect(find("view")!.parentElement!.parentElement).toBe(find("content"));
    });

    it("holds a view that reads as it mounts the same way: it is in the document while its first read is pending", async () => {
        const pending = pendingRead<string>();
        const { container } = render(() => (
            <PaneViewBoundary fallback={<span data-testid="pending">pending</span>}>
                <Late read={pending.read} />
            </PaneViewBoundary>
        ));
        const find = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);
        expect(find("pending")).not.toBeNull();
        const late = find("late")!;
        expect(late.isConnected).toBe(true);
        await pending.settle("read");
        expect(find("pending")).toBeNull();
        expect(find("late")).toBe(late);
        expect(late.textContent).toBe("read");
    });

    it("takes the view out when the boundary itself goes", () => {
        const [shown, setShown] = createSignal(true);
        const { container } = render(() => (
            <Show when={shown()}>
                <PaneViewBoundary fallback={<span />}>
                    <div data-testid="view" />
                </PaneViewBoundary>
            </Show>
        ));
        const view = container.querySelector<HTMLElement>('[data-testid="view"]')!;
        expect(view.isConnected).toBe(true);
        setShown(false);
        expect(view.isConnected).toBe(false);
        expect(container.querySelector(".pane-view-host")).toBeNull();
    });
});
