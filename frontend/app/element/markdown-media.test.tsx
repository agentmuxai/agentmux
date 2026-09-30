// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Inline images in the agent's own messages
 * (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §4, P3): local files load through
 * the Media pane's authed path into a blob URL, lazily and under a size cap;
 * remote images wait for a click; nothing loads where `media` isn't passed
 * (tool results, file previews); and raw `<picture><source>` can't smuggle a
 * remote fetch in beside a local image.
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    fetch: vi.fn(),
    createBlock: vi.fn(async () => "new-block"),
}));

vi.mock("@/util/fetchutil", () => ({ fetch: (...a: unknown[]) => h.fetch(...a) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/app/store/block-layout-actions", () => ({ createBlock: h.createBlock }));

import { Markdown } from "./markdown";
import { resolveMediaPath } from "./local-media";

const BASE = "C:/work";
let urls = 0;
const revoked: string[] = [];

function okResponse(bytes = 10, type = "image/png") {
    return {
        ok: true,
        status: 200,
        statusText: "OK",
        headers: new Headers({ "Content-Length": String(bytes), "Content-Type": type }),
        blob: async () => new Blob([new Uint8Array(bytes)], { type }),
        body: { cancel: async () => {} },
    };
}

beforeEach(() => {
    h.fetch.mockReset();
    h.createBlock.mockClear();
    h.fetch.mockImplementation(async () => okResponse());
    urls = 0;
    revoked.length = 0;
    vi.stubGlobal("URL", Object.assign(URL, {
        createObjectURL: () => `blob:test/${++urls}`,
        revokeObjectURL: (u: string) => revoked.push(u),
    }));
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

/** `null`: media off, as in a tool result or file preview. */
const mount = (text: string, media: { baseDir: string } | null = { baseDir: BASE }) =>
    render(() => <Markdown text={text} scrollable={false} media={media ?? undefined} />);

const requestedPath = (call = 0) => new URL(h.fetch.mock.calls[call][0] as string).searchParams.get("path");

describe("resolveMediaPath", () => {
    it.each([
        ["shots/a.png", "C:/work/shots/a.png"],
        ["./a.png", "C:/work/a.png"],
        ["C:/pics/a.png", "C:/pics/a.png"],
        ["C:\\pics\\a.png", "C:\\pics\\a.png"],
        ["/home/me/a.png", "/home/me/a.png"],
        ["~/a.png", "~/a.png"],
        ["file:///C:/pics/a.png", "C:/pics/a.png"],
        ["file:///home/me/a.png", "/home/me/a.png"],
        ["my%20shot.png", "C:/work/my shot.png"],
    ])("%s → %s", (src, want) => {
        expect(resolveMediaPath(src, BASE)).toBe(want);
    });

    it("joins against a base dir with a trailing or back slash", () => {
        expect(resolveMediaPath("a.png", "C:\\work\\")).toBe("C:\\work/a.png");
    });

    it("can't resolve a relative path without a base dir", () => {
        expect(resolveMediaPath("a.png", "")).toBeNull();
    });
});

describe("a local image in an agent message", () => {
    it("fetches the file with the auth header and shows it from a blob URL, alt text as caption", async () => {
        const c = mount("Here: ![the new toolbar](shots/a.png)").container;
        await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
        expect(requestedPath()).toBe("C:/work/shots/a.png");
        expect(new URL(h.fetch.mock.calls[0][0] as string).pathname).toBe("/agentmux/stream-local-file");
        expect((h.fetch.mock.calls[0][1] as RequestInit).headers).toMatchObject({ "X-AuthKey": "k" });
        expect(c.querySelector("img")!.getAttribute("src")).toMatch(/^blob:/);
        expect(c.querySelector("figcaption")?.textContent).toBe("the new toolbar");
    });

    it("loads a Windows absolute path (the sanitizer would read C: as a URL scheme)", async () => {
        const c = mount("![x](C:/pics/a.png)").container;
        await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
        expect(requestedPath()).toBe("C:/pics/a.png");
    });

    it("loads a backslash Windows path", async () => {
        const c = mount("![x](C:\\pics\\a.png)").container;
        await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
        expect(requestedPath()).toBe("C:/pics/a.png");
    });

    it("loads a path with spaces written in <…>", async () => {
        const c = mount("![x](<my shots/a b.png>)").container;
        await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
        expect(requestedPath()).toBe("C:/work/my shots/a b.png");
    });

    it("serves SVG as image/svg+xml so it renders (its scripts never run from an <img>)", async () => {
        h.fetch.mockImplementation(async () => okResponse(10, "application/octet-stream"));
        let type = "";
        vi.stubGlobal("URL", Object.assign(URL, {
            createObjectURL: (b: Blob) => ((type = b.type), "blob:test/svg"),
            revokeObjectURL: () => {},
        }));
        const c = mount("![x](diagram.svg)").container;
        await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
        expect(type).toBe("image/svg+xml");
    });

    it.each(["tool.exe", "notes.txt"])("makes no request for %s", (file) => {
        const c = mount(`![x](${file})`).container;
        expect(h.fetch).not.toHaveBeenCalled();
        expect(c.textContent).toContain(`[image: ${file} — unsupported type]`);
    });

    it("shows a muted note, not a broken image, when the file is missing", async () => {
        h.fetch.mockImplementation(async () => ({ ok: false, status: 404, statusText: "Not Found", headers: new Headers() }));
        const c = mount("![x](gone.png)").container;
        await waitFor(() => expect(c.querySelector(".am-muted")?.textContent).toBe("[image not found: gone.png]"));
        expect(c.querySelector("img")).toBeNull();
    });

    it("names a missing Windows path as written, not as the internal file:/// URL", async () => {
        h.fetch.mockImplementation(async () => ({ ok: false, status: 404, statusText: "Not Found", headers: new Headers() }));
        const c = mount("![x](C:/pics/gone.png)").container;
        await waitFor(() => expect(c.querySelector(".am-muted")?.textContent).toBe("[image not found: C:/pics/gone.png]"));
    });

    it("sets the image's aspect ratio from its natural size, so the row settles once", async () => {
        const proto = HTMLImageElement.prototype as any;
        vi.spyOn(proto, "naturalWidth", "get").mockReturnValue(400);
        vi.spyOn(proto, "naturalHeight", "get").mockReturnValue(300);
        proto.decode = () => Promise.resolve();
        try {
            const c = mount("![x](ratio.png)").container;
            await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
            expect(c.querySelector<HTMLElement>("img")!.style.aspectRatio).toBe("400 / 300");
        } finally {
            delete proto.decode;
            vi.restoreAllMocks();
        }
    });

    it("shows the image anyway when decoding never finishes", async () => {
        const proto = HTMLImageElement.prototype as any;
        proto.decode = () => new Promise(() => {});
        try {
            const c = mount("![x](slow.png)").container;
            await waitFor(() => expect(c.querySelector("img")).not.toBeNull(), { timeout: 3000 });
        } finally {
            delete proto.decode;
        }
    });

    it("revokes the blob URL on unmount", async () => {
        const r = mount("![x](a.png)");
        await waitFor(() => expect(r.container.querySelector("img")).not.toBeNull());
        const src = r.container.querySelector("img")!.getAttribute("src")!;
        r.unmount();
        expect(revoked).toContain(src);
    });

    it("waits until the placeholder nears the viewport", async () => {
        const observers: { cb: IntersectionObserverCallback; el?: Element }[] = [];
        vi.stubGlobal(
            "IntersectionObserver",
            class {
                o: { cb: IntersectionObserverCallback; el?: Element };
                constructor(cb: IntersectionObserverCallback) {
                    this.o = { cb };
                    observers.push(this.o);
                }
                observe(el: Element) {
                    this.o.el = el;
                }
                disconnect() {}
                unobserve() {}
            },
        );
        const c = mount("![x](a.png)").container;
        expect(c.querySelector(".am-media-placeholder")).not.toBeNull();
        expect(h.fetch).not.toHaveBeenCalled();
        const o = observers[0];
        o.cb([{ isIntersecting: true, target: o.el! } as IntersectionObserverEntry], {} as IntersectionObserver);
        await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
        expect(h.fetch).toHaveBeenCalledTimes(1);
    });

    it("opens the file in a Media pane on click", async () => {
        const c = mount("![x](shots/a.png)").container;
        await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
        fireEvent.click(c.querySelector("img")!);
        expect(h.createBlock).toHaveBeenCalledWith({ meta: { view: "media", "media:path": "C:/work/shots/a.png" } });
    });

    it("shows a card instead of an image over the 25 MB cap, without downloading it", async () => {
        const blob = vi.fn();
        h.fetch.mockImplementation(async () => ({ ...okResponse(), headers: new Headers({ "Content-Length": String(30 * 1024 * 1024) }), blob }));
        const c = mount("![x](shots/huge.png)").container;
        await waitFor(() => expect(c.querySelector(".am-media-card")).not.toBeNull());
        expect(blob).not.toHaveBeenCalled();
        expect(c.querySelector("img")).toBeNull();
        const card = c.querySelector(".am-media-card")!;
        expect(card.textContent).toContain("huge.png");
        expect(card.textContent).toContain("30 MB");
        fireEvent.click(card);
        expect(h.createBlock).toHaveBeenCalledWith({ meta: { view: "media", "media:path": "C:/work/shots/huge.png" } });
    });
});

describe("remote images wait for a click", () => {
    it("renders a chip naming the host and loads only when clicked", () => {
        const c = mount("![x](https://example.com/p.png?d=secret)").container;
        expect(c.querySelector("img")).toBeNull();
        const chip = c.querySelector(".am-media-chip")!;
        expect(chip.textContent).toContain("image from example.com");
        fireEvent.click(chip);
        expect(c.querySelector("img")?.getAttribute("src")).toBe("https://example.com/p.png?d=secret");
        expect(h.fetch).not.toHaveBeenCalled();
    });

    it("warns that http:// isn't encrypted", () => {
        const c = mount("![x](http://example.com/p.png)").container;
        expect(c.querySelector(".am-media-chip")!.textContent).toContain("not encrypted");
    });

    it("renders a raster data: image directly", () => {
        const c = mount("![x](data:image/png;base64,iVBORw0KGgo=)").container;
        expect(c.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,iVBORw0KGgo=");
    });

    // ReAgent P0 on #4064: an SVG can reference remote resources, so an inline
    // SVG data URI never renders; a local .svg file is shown the same way the
    // user's own files are, from a blob through <img>.
    it("doesn't render an SVG data: URI", () => {
        const c = mount("![x](data:image/svg+xml;base64,PHN2Zy8+)").container;
        expect(c.querySelector("img")).toBeNull();
        expect(c.textContent).toContain("unsupported type");
    });
});

describe("nothing loads where media isn't enabled (tool results, file previews)", () => {
    it.each([
        "![x](a.png)",
        "![x](C:/pics/a.png)",
        "![x](https://example.com/p.png)",
        "![x](data:image/png;base64,iVBORw0KGgo=)",
        "![x](data:image/svg+xml;base64,PHN2Zy8+)",
    ])("%s", (md) => {
        const c = mount(md, null).container;
        expect(c.querySelector("img")).toBeNull();
        expect(c.querySelector(".am-media-chip")).toBeNull();
        expect(h.fetch).not.toHaveBeenCalled();
    });
});

describe("raw <picture>/<source>", () => {
    it("is stripped, so a remote srcset can't load beside a local image", async () => {
        const c = mount(`<picture><source srcset="https://evil.example/x.png"><img src="a.png"></picture>`).container;
        await waitFor(() => expect(c.querySelector("img")).not.toBeNull());
        expect(c.querySelector("source, picture")).toBeNull();
        expect(c.querySelector("img")!.getAttribute("srcset")).toBeNull();
    });
});

describe("Operator Config tells agents about images", () => {
    const manifest = JSON.parse(
        readFileSync(join(__dirname, "../../../agentmux-srv/operator-config-seed.json"), "utf8"),
    ) as { version: number; entries: { id: string; instructions: string }[] };
    const entry = manifest.entries.find((e) => e.id === "operator-config-rich-output")!;

    it("describes local images and is in a newer manifest generation than P2's", () => {
        expect(manifest.version).toBeGreaterThanOrEqual(4);
        expect(entry.instructions).toMatch(/!\[[^\]]*\]\([^)]*\.png\)/);
    });
});
