import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/misc-utils", () => ({ isDev: () => false }));
vi.mock("./locales/en.json", () => ({
    default: {
        "t.plain": "Just text",
        "t.mixed": "<strong>Google:</strong> your name (the <code>openid</code> permission).",
        "t.name": "Connected as <strong>{{who}}</strong> today",
        "t.keys.one": "The key is in <strong>{{place}}</strong>.",
        "t.keys.other": "The {{count}} keys are in <strong>{{place}}</strong>.",
    },
}));

import { richParts, Rich } from "./rich";

describe("richParts", () => {
    it("splits plain text and tagged runs, and keeps another < as text", () => {
        expect(richParts("a <strong>b</strong> c <em>d</em> 1 < 2")).toEqual([
            { text: "a " },
            { tag: "strong", text: "b" },
            { text: " c " },
            { tag: "em", text: "d" },
            { text: " 1 < 2" },
        ]);
        expect(richParts("plain")).toEqual([{ text: "plain" }]);
        expect(richParts("")).toEqual([]);
    });

    it("does not read a tag outside the three", () => {
        expect(richParts("<script>x</script>")).toEqual([{ text: "<script>x</script>" }]);
    });
});

describe("Rich", () => {
    afterEach(() => cleanup());

    it("renders the tags as elements", () => {
        const { container } = render(() => <Rich k="t.mixed" />);
        expect(container.querySelector("strong")?.textContent).toBe("Google:");
        expect(container.querySelector("code")?.textContent).toBe("openid");
        expect(container.textContent).toBe("Google: your name (the openid permission).");
    });

    it("fills a name inside a tag, and what fills it never becomes markup", () => {
        const { container } = render(() => <Rich k="t.name" params={{ who: "<strong>x</strong>@a.b" }} />);
        const strong = container.querySelectorAll("strong");
        expect(strong).toHaveLength(1);
        expect(strong[0].textContent).toBe("<strong>x</strong>@a.b");
        expect(container.querySelector("strong strong")).toBeNull();
    });

    it("takes the plural variant a count picks, and still fills the names", () => {
        const one = render(() => <Rich k="t.keys" params={{ count: 1, place: "eu-west-1" }} />);
        expect(one.container.textContent).toBe("The key is in eu-west-1.");
        expect(one.container.querySelector("strong")?.textContent).toBe("eu-west-1");
        cleanup();
        const many = render(() => <Rich k="t.keys" params={{ count: 3, place: "<em>x</em>" }} />);
        expect(many.container.textContent).toBe("The 3 keys are in <em>x</em>.");
        expect(many.container.querySelector("em")).toBeNull();
    });

    it("renders plain text", () => {
        const { container } = render(() => <Rich k="t.plain" />);
        expect(container.textContent).toBe("Just text");
    });
});
