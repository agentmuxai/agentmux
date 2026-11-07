// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for UserInputModal: what an srv input request shows (title with its
 * countdown, the query as text or Markdown, the text box, the checkbox, the two buttons), what each
 * way of answering sends (OK, Cancel, Enter, Escape/close, the timeout) for a "text" and a "confirm"
 * request, and the countdown's timers. They pin today's behaviour (written against the unsplit
 * component) so it can be split into named pieces without changing it. The modal shell, the buttons,
 * the text input, the Markdown renderer and the service are stubs that record what they were given;
 * the key matching and the DOM are real.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import type { JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    modal: null as null | { open?: boolean; onClose?: () => void; size?: string },
    send: vi.fn<(resp: Record<string, unknown>) => Promise<void>>(async () => {}),
    textInputProps: null as null | Record<string, unknown>,
}));

vi.mock("@/element/modal", () => ({
    Modal: (props: { open?: boolean; onClose?: () => void; size?: string; children?: JSX.Element }) => {
        h.modal = props;
        return <div data-testid="modal">{props.children}</div>;
    },
    ModalHeader: (props: { title: string }) => <div data-testid="header">{props.title}</div>,
    ModalBody: (props: { children?: JSX.Element }) => <div data-testid="body">{props.children}</div>,
    ModalFooter: (props: { children?: JSX.Element }) => <div data-testid="footer">{props.children}</div>,
}));
vi.mock("@/element/button", () => ({
    Button: (props: { onClick?: () => void; children?: JSX.Element }) => (
        <button data-testid="button" onClick={() => props.onClick?.()}>
            {props.children}
        </button>
    ),
}));
vi.mock("@/app/element/ui", () => ({
    TextInput: (props: Record<string, unknown>) => {
        h.textInputProps = props;
        return (
            <input
                data-testid="text-input"
                data-variant={String(props.variant)}
                type={props.type as string}
                value={props.value as string}
                maxLength={props.maxLength as number}
                class={props.class as string}
                onInput={props.onInput as JSX.EventHandler<HTMLInputElement, InputEvent>}
                onKeyDown={props.onKeyDown as JSX.EventHandler<HTMLInputElement, KeyboardEvent>}
            />
        );
    },
}));
vi.mock("@/element/markdown", () => ({
    Markdown: (props: { text: string; class?: string }) => (
        <div data-testid="markdown" class={props.class}>
            {props.text}
        </div>
    ),
}));
vi.mock("../store/services", () => ({ UserInputService: { SendUserInputResponse: h.send } }));

import { UserInputModal } from "./userinputmodal";

function request(overrides: Partial<UserInputRequest> = {}): UserInputRequest {
    return {
        requestid: "req-1",
        querytext: "What is your name?",
        responsetype: "text",
        title: "Question",
        markdown: false,
        timeoutms: 30000,
        checkboxmsg: "",
        publictext: true,
        ...overrides,
    };
}

function mount(overrides: Partial<UserInputRequest> = {}, close = vi.fn()) {
    const utils = render(() => <UserInputModal {...request(overrides)} close={close} />);
    return { ...utils, close };
}

function buttons(container: HTMLElement): HTMLButtonElement[] {
    return Array.from(container.querySelectorAll<HTMLButtonElement>("[data-testid=button]"));
}

async function flush(): Promise<void> {
    for (let i = 0; i < 5; i++) await Promise.resolve();
}

beforeEach(() => {
    vi.useFakeTimers();
    h.modal = null;
    h.textInputProps = null;
    h.send.mockReset().mockImplementation(async () => {});
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("UserInputModal render", () => {
    it("is named for the dev tools", () => {
        expect(UserInputModal.displayName).toBe("UserInputModal");
    });

    it("opens a medium modal that closes with a cancel answer", () => {
        mount();
        expect(h.modal?.open).toBe(true);
        expect(h.modal?.size).toBe("md");
        expect(typeof h.modal?.onClose).toBe("function");
    });

    it("lays out the header, the body and the footer in that order", () => {
        const { getByTestId } = mount();
        const modal = getByTestId("modal");
        expect(Array.from(modal.children).map((c) => c.getAttribute("data-testid"))).toEqual([
            "header",
            "body",
            "footer",
        ]);
    });

    it("titles the modal with the request title and the seconds left", () => {
        const { getByTestId } = mount({ title: "Sign in", timeoutms: 45000 });
        expect(getByTestId("header").textContent).toBe("Sign in (45s)");
    });

    it("rounds the countdown down to whole seconds", () => {
        const { getByTestId } = mount({ timeoutms: 4999 });
        expect(getByTestId("header").textContent).toBe("Question (4s)");
    });

    it("keeps the body's query, input and checkbox in a userinput-body", () => {
        const { getByTestId } = mount({ checkboxmsg: "Remember" });
        const body = getByTestId("body").firstElementChild as HTMLElement;
        expect(body.className).toBe("userinput-body");
        expect(body.children[0].className).toBe("userinput-text");
        expect(body.children[1].getAttribute("data-testid")).toBe("text-input");
        expect(body.children[2].className).toBe("userinput-checkbox-container");
    });

    it("shows a plain query as text", () => {
        const { container, queryByTestId } = mount({ querytext: "Plain *text*" });
        const span = container.querySelector("span.userinput-text") as HTMLElement;
        expect(span.textContent).toBe("Plain *text*");
        expect(queryByTestId("markdown")).toBeNull();
    });

    it("shows a Markdown query through the Markdown renderer", () => {
        const { container, getByTestId } = mount({ markdown: true, querytext: "Plain *text*" });
        const md = getByTestId("markdown");
        expect(md.textContent).toBe("Plain *text*");
        expect(md.className).toBe("userinput-markdown");
        expect(container.querySelector("span.userinput-text")).toBeNull();
    });
});

describe("UserInputModal text box", () => {
    it("is a plain, focused, 400-character box for a text request", () => {
        const { getByTestId } = mount();
        const input = getByTestId("text-input") as HTMLInputElement;
        expect(input.getAttribute("data-variant")).toBe("plain");
        expect(input.className).toBe("userinput-inputbox");
        expect(input.maxLength).toBe(400);
        expect(h.textInputProps?.autofocus).toBe(true);
    });

    it("shows what is typed for public text and hides it otherwise", () => {
        const first = mount({ publictext: true });
        expect((first.getByTestId("text-input") as HTMLInputElement).type).toBe("text");
        cleanup();
        const second = mount({ publictext: false });
        expect((second.getByTestId("text-input") as HTMLInputElement).type).toBe("password");
    });

    it("has no box at all for a confirm request", () => {
        const { queryByTestId } = mount({ responsetype: "confirm" });
        expect(queryByTestId("text-input")).toBeNull();
    });

    it("keeps the typed text as its value", () => {
        const { getByTestId } = mount();
        const input = getByTestId("text-input") as HTMLInputElement;
        input.value = "Maop";
        fireEvent.input(input);
        expect(h.textInputProps?.value).toBe("Maop");
        expect(input.value).toBe("Maop");
    });
});

describe("UserInputModal checkbox", () => {
    it("is absent when the message is empty", () => {
        const { container } = mount({ checkboxmsg: "" });
        expect(container.querySelector("input[type=checkbox]")).toBeNull();
        expect(container.querySelector(".userinput-checkbox-container")).toBeNull();
    });

    it("shows an unchecked box with its label, tied together by the request id", () => {
        const { container } = mount({ checkboxmsg: "Remember this", requestid: "r-9" });
        const box = container.querySelector("input[type=checkbox]") as HTMLInputElement;
        const label = container.querySelector("label") as HTMLLabelElement;
        expect(box.id).toBe("uicheckbox-r-9");
        expect(box.className).toBe("userinput-checkbox");
        expect(box.checked).toBe(false);
        expect(label.getAttribute("for")).toBe("uicheckbox-r-9");
        expect(label.textContent).toBe("Remember this");
        expect(box.closest(".userinput-checkbox-row")).not.toBeNull();
        expect(box.closest(".userinput-checkbox-container")).not.toBeNull();
    });

    it("is shown for a request that leaves the message out", () => {
        const { container } = mount({ checkboxmsg: undefined as unknown as string });
        expect(container.querySelector("input[type=checkbox]")).not.toBeNull();
    });

    it("is offered on a confirm request too", () => {
        const { container } = mount({ responsetype: "confirm", checkboxmsg: "Always" });
        expect(container.querySelector("input[type=checkbox]")).not.toBeNull();
    });
});

describe("UserInputModal buttons", () => {
    it("are Cancel then Ok by default", () => {
        const { container } = mount();
        expect(buttons(container).map((b) => b.textContent)).toEqual(["Cancel", "Ok"]);
    });

    it("take the request's own labels", () => {
        const { container } = mount({ cancellabel: "No thanks", oklabel: "Yes please" });
        expect(buttons(container).map((b) => b.textContent)).toEqual(["No thanks", "Yes please"]);
    });

    it("keep an empty label rather than falling back", () => {
        const { container } = mount({ cancellabel: "", oklabel: "" });
        expect(buttons(container).map((b) => b.textContent)).toEqual(["", ""]);
    });

    it("sit in the footer", () => {
        const { getByTestId } = mount();
        expect(getByTestId("footer").querySelectorAll("[data-testid=button]").length).toBe(2);
    });
});

describe("UserInputModal answers", () => {
    it("sends the typed text and the checkbox state on Ok for a text request, then closes", async () => {
        const { container, getByTestId, close } = mount({ requestid: "r-1", checkboxmsg: "Remember" });
        const input = getByTestId("text-input") as HTMLInputElement;
        input.value = "hunter2";
        fireEvent.input(input);
        (container.querySelector("input[type=checkbox]") as HTMLInputElement).checked = true;
        buttons(container)[1].click();
        await flush();
        expect(h.send).toHaveBeenCalledTimes(1);
        expect(h.send).toHaveBeenCalledWith({
            type: "userinputresp",
            requestid: "r-1",
            text: "hunter2",
            checkboxstat: true,
        });
        expect(close).toHaveBeenCalledTimes(1);
    });

    it("sends an empty text and false when nothing was typed or ticked", async () => {
        const { container } = mount({ checkboxmsg: "Remember" });
        buttons(container)[1].click();
        await flush();
        expect(h.send).toHaveBeenCalledWith({
            type: "userinputresp",
            requestid: "req-1",
            text: "",
            checkboxstat: false,
        });
    });

    it("sends checkbox false when the request has no checkbox", async () => {
        const { container } = mount();
        buttons(container)[1].click();
        await flush();
        expect(h.send.mock.calls[0][0].checkboxstat).toBe(false);
    });

    it("sends confirm true and the checkbox state on Ok for a confirm request, then closes", async () => {
        const { container, close } = mount({ responsetype: "confirm", checkboxmsg: "Always" });
        (container.querySelector("input[type=checkbox]") as HTMLInputElement).checked = true;
        buttons(container)[1].click();
        await flush();
        expect(h.send).toHaveBeenCalledWith({
            type: "userinputresp",
            requestid: "req-1",
            confirm: true,
            checkboxstat: true,
        });
        expect(close).toHaveBeenCalledTimes(1);
    });

    it("sends checkbox false on a confirm request that has no checkbox", async () => {
        const { container } = mount({ responsetype: "confirm" });
        buttons(container)[1].click();
        buttons(container)[0].click();
        await flush();
        expect(h.send.mock.calls.map((c) => c[0].checkboxstat)).toEqual([false, false]);
    });

    it("sends confirm false on Cancel for a confirm request, then closes", async () => {
        const { container, close } = mount({ responsetype: "confirm", checkboxmsg: "Always" });
        (container.querySelector("input[type=checkbox]") as HTMLInputElement).checked = true;
        buttons(container)[0].click();
        await flush();
        expect(h.send).toHaveBeenCalledWith({
            type: "userinputresp",
            requestid: "req-1",
            confirm: false,
            checkboxstat: true,
        });
        expect(close).toHaveBeenCalledTimes(1);
    });

    it("sends a cancelled error on Cancel for a text request, then closes", async () => {
        const { container, close } = mount({ requestid: "r-4" });
        buttons(container)[0].click();
        await flush();
        expect(h.send).toHaveBeenCalledTimes(1);
        expect(h.send).toHaveBeenCalledWith({
            type: "userinputresp",
            requestid: "r-4",
            errormsg: "Canceled by the user",
        });
        expect(close).toHaveBeenCalledTimes(1);
    });

    it("sends the cancelled error when the modal is closed, even for a confirm request", async () => {
        const { close } = mount({ responsetype: "confirm", requestid: "r-5" });
        h.modal?.onClose?.();
        await flush();
        expect(h.send).toHaveBeenCalledWith({
            type: "userinputresp",
            requestid: "r-5",
            errormsg: "Canceled by the user",
        });
        expect(close).toHaveBeenCalledTimes(1);
    });

    it("sends nothing and stays open for a request type it does not know", async () => {
        const { container, close } = mount({ responsetype: "other" });
        buttons(container)[0].click();
        buttons(container)[1].click();
        await flush();
        expect(h.send).not.toHaveBeenCalled();
        expect(close).not.toHaveBeenCalled();
    });

    it("closes even though the service call fails", async () => {
        h.send.mockRejectedValueOnce(new Error("srv down"));
        const log = vi.spyOn(console, "log").mockImplementation(() => {});
        const { container, close } = mount();
        buttons(container)[1].click();
        await flush();
        expect(close).toHaveBeenCalledTimes(1);
        expect(log).toHaveBeenCalledWith("fireAndForget error", expect.any(Error));
        log.mockRestore();
    });
});

describe("UserInputModal Enter key", () => {
    function press(input: HTMLElement, key: string) {
        const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
        input.dispatchEvent(event);
        return event;
    }

    it("submits the text, swallowing the key", async () => {
        const { getByTestId, close } = mount({ requestid: "r-6" });
        const input = getByTestId("text-input") as HTMLInputElement;
        input.value = "typed";
        fireEvent.input(input);
        const event = press(input, "Enter");
        await flush();
        expect(event.defaultPrevented).toBe(true);
        expect(h.send).toHaveBeenCalledWith({
            type: "userinputresp",
            requestid: "r-6",
            text: "typed",
            checkboxstat: false,
        });
        expect(close).toHaveBeenCalledTimes(1);
    });

    it("lets other keys through", async () => {
        const { getByTestId, close } = mount();
        const event = press(getByTestId("text-input"), "a");
        await flush();
        expect(event.defaultPrevented).toBe(false);
        expect(h.send).not.toHaveBeenCalled();
        expect(close).not.toHaveBeenCalled();
    });

    it("does not treat Escape as its own (the modal's close handles it)", async () => {
        const { getByTestId } = mount();
        press(getByTestId("text-input"), "Escape");
        await flush();
        expect(h.send).not.toHaveBeenCalled();
    });
});

describe("UserInputModal countdown", () => {
    it("ticks down once a second", () => {
        const { getByTestId } = mount({ timeoutms: 5000 });
        vi.advanceTimersByTime(1000);
        expect(getByTestId("header").textContent).toBe("Question (4s)");
        vi.advanceTimersByTime(2000);
        expect(getByTestId("header").textContent).toBe("Question (2s)");
    });

    it("sends the cancelled error 300 ms after it reaches zero, then closes", async () => {
        const { getByTestId, close } = mount({ timeoutms: 2000, requestid: "r-7" });
        vi.advanceTimersByTime(2000);
        expect(getByTestId("header").textContent).toBe("Question (0s)");
        expect(h.send).not.toHaveBeenCalled();
        vi.advanceTimersByTime(299);
        expect(h.send).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        await flush();
        expect(h.send).toHaveBeenCalledTimes(1);
        expect(h.send).toHaveBeenCalledWith({
            type: "userinputresp",
            requestid: "r-7",
            errormsg: "Canceled by the user",
        });
        expect(close).toHaveBeenCalledTimes(1);
    });

    it("stops ticking once it reaches zero", async () => {
        const { getByTestId } = mount({ timeoutms: 1000 });
        vi.advanceTimersByTime(1000);
        vi.advanceTimersByTime(10000);
        await flush();
        expect(getByTestId("header").textContent).toBe("Question (0s)");
        expect(h.send).toHaveBeenCalledTimes(1);
    });

    it("runs out on the first tick when it starts at zero or one second", async () => {
        mount({ timeoutms: 0 });
        vi.advanceTimersByTime(1000);
        vi.advanceTimersByTime(300);
        await flush();
        expect(h.send).toHaveBeenCalledTimes(1);
    });

    it("does not tick before the first second", () => {
        const { getByTestId } = mount({ timeoutms: 5000 });
        vi.advanceTimersByTime(999);
        expect(getByTestId("header").textContent).toBe("Question (5s)");
    });

    it("stops the tick when the modal goes away", () => {
        const { getByTestId, unmount } = mount({ timeoutms: 5000 });
        const header = getByTestId("header");
        unmount();
        vi.advanceTimersByTime(10000);
        expect(header.textContent).toBe("Question (5s)");
        expect(h.send).not.toHaveBeenCalled();
    });

    it("drops the pending cancel when the modal goes away inside the 300 ms window", async () => {
        const { unmount, close } = mount({ timeoutms: 1000 });
        vi.advanceTimersByTime(1000);
        unmount();
        vi.advanceTimersByTime(1000);
        await flush();
        expect(h.send).not.toHaveBeenCalled();
        expect(close).not.toHaveBeenCalled();
    });
});
