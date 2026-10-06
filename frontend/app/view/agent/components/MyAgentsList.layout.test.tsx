// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * My Agents tile layout, measured in a real browser.
 *
 * jsdom has no layout, so the bug this guards (the chevron and the panel it
 * opens landing at the bottom of the tallest tile in the row instead of at the
 * bottom of their own tile) cannot be seen by any DOM or class-name assertion.
 * This test renders the real `MyAgentsList` markup, compiles the real
 * `_recent-sessions.scss`, loads both in headless Chromium (Chrome or Edge,
 * whichever is installed) and measures the result.
 *
 * It skips, rather than fails, on a machine with no Chromium-based browser.
 * Set `AGENTMUX_TEST_BROWSER` to a browser executable to force one.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import * as sass from "sass";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

import { MyAgentsList } from "./MyAgentsList";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListRecentSessionsCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// A fixed-size box where the 40 px provider logo goes, so the tile has its real
// icon column.
vi.mock("@/element/DualProviderLogo", () => ({
    DualProviderLogo: (props: { class?: string }) => (
        <span class={props.class} style={{ display: "block", width: "40px", height: "40px" }} />
    ),
}));

function findBrowser(): string | null {
    const candidates = [
        process.env.AGENTMUX_TEST_BROWSER,
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
        "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ];
    return candidates.find((c): c is string => !!c && existsSync(c)) ?? null;
}

const BROWSER = findBrowser();

/** The app's design tokens that the tile CSS reads, with their real values. */
const TOKENS = `:root{
  --space-1:4px;--space-1-5:6px;--space-2:8px;--space-3:12px;--space-4:16px;
  --text-xs:11px;--text-sm:12px;
  --main-text-color:#e0e0e0;--secondary-text-color:#9a9a9a;--border-color:#444;
  --accent-color:#58c142;--error-color:#f87171;--modal-bg-color:#222;
}
body{margin:0;background:#111;font-family:sans-serif}`;

/** Runs in the page: measures every tile and what a click at three points would hit. */
const MEASURE = `
const box = (el) => { if (!el) return null; const r = el.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, height: r.height, width: r.width }; };
const hitAt = (x, y) => { const el = document.elementFromPoint(x, y); return el ? (el.closest('.agent-recent-sessions-menu-toggle') ? 'toggle' : el.closest('.agent-recent-sessions-entry') ? 'entry' : el.className || el.tagName) : null; };
const centre = (r) => [(r.left + r.right) / 2, (r.top + r.bottom) / 2];
const tiles = [...document.querySelectorAll('.agent-recent-sessions-row')].map((li) => {
  const entry = li.querySelector('.agent-recent-sessions-entry');
  const toggle = li.querySelector('.agent-recent-sessions-menu-toggle');
  const stamps = li.querySelector('.agent-recent-sessions-timestamps');
  const liBox = box(li), toggleBox = box(toggle), stampsBox = box(stamps);
  return {
    name: li.querySelector('.agent-recent-sessions-name')?.textContent,
    li: liBox, entry: box(entry), toggle: toggleBox, stamps: stampsBox,
    liBorderTop: parseFloat(getComputedStyle(li).borderTopWidth),
    hitCentre: hitAt(...centre(liBox)),
    hitStamps: stampsBox ? hitAt(...centre(stampsBox)) : null,
    hitToggle: hitAt(...centre(toggleBox)),
  };
});
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(tiles);
document.body.appendChild(out);`;

interface Box {
    top: number;
    bottom: number;
    left: number;
    right: number;
    height: number;
    width: number;
}
interface Tile {
    name: string;
    li: Box;
    entry: Box;
    toggle: Box;
    stamps: Box | null;
    liBorderTop: number;
    hitCentre: string;
    hitStamps: string | null;
    hitToggle: string;
}

function measureInBrowser(bodyHtml: string): Tile[] {
    const css = sass.compile(join(__dirname, "..", "styles", "_recent-sessions.scss")).css;
    const dir = mkdtempSync(join(tmpdir(), "agentmux-tile-layout-"));
    try {
        const file = join(dir, "fixture.html");
        writeFileSync(
            file,
            `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>` +
                `<div class="agent-view" style="width:1000px;padding:16px"><div class="agent-picker">${bodyHtml}</div></div>` +
                `<script>window.addEventListener("load",()=>{${MEASURE}})</script>`
        );
        const dom = execFileSync(
            BROWSER!,
            [
                "--headless=new",
                "--disable-gpu",
                "--no-sandbox",
                "--hide-scrollbars",
                `--user-data-dir=${join(dir, "profile")}`,
                "--window-size=1100,1200",
                "--virtual-time-budget=3000",
                "--dump-dom",
                pathToFileURL(file).href,
            ],
            { encoding: "utf8", timeout: 60_000, maxBuffer: 16 * 1024 * 1024 }
        );
        const m = /<pre id="result">([\s\S]*?)<\/pre>/.exec(dom);
        if (!m) throw new Error("the page produced no measurements:\n" + dom.slice(0, 2000));
        return JSON.parse(
            m[1]
                .replace(/&quot;/g, '"')
                .replace(/&amp;/g, "&")
                .replace(/&lt;/g, "<")
                .replace(/&gt;/g, ">")
        );
    } finally {
        rmSync(dir, { recursive: true, force: true });
    }
}

/** Tiles whose heights differ: a three-line summary, none, one line, and a sandbox badge. */
const rows = [
    {
        instance_id: "a",
        instance_name: "Long summary",
        definition_id: "def-a",
        preview: "word ".repeat(120),
        identity_name: "work@example.com",
        node_count: 40,
    },
    {
        instance_id: "b",
        instance_name: "Empty",
        definition_id: "def-b",
        preview: "",
        identity_name: "",
        node_count: 0,
        has_snapshot: false,
    },
    {
        instance_id: "c",
        instance_name: "Short",
        definition_id: "def-c",
        preview: "one line",
        identity_name: "No auth",
        node_count: 3,
    },
    {
        instance_id: "d",
        instance_name: "Sandboxed",
        definition_id: "def-d",
        preview: "word ".repeat(22),
        identity_name: "work@example.com",
        node_count: 9,
        agent_type: "container",
    },
];

let tiles: Tile[] = [];

describe.skipIf(!BROWSER)("My Agents tile layout (real browser)", () => {
    beforeAll(async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue({
            rows: rows.map((r) => ({
                provider: "claude",
                definition_name: "Claude Code",
                working_directory: "/tmp",
                identity_id: "id",
                memory_id: "m",
                memory_name: "m",
                block_id_hint: "blk",
                last_active_at: Date.now() - 60_000,
                has_snapshot: true,
                agent_created_at: Date.now() - 7_776_000_000,
                started_at: Date.now() - 3_600_000,
                ...r,
            })),
            degraded: [],
        } as never);
        render(() => <MyAgentsList onReattach={() => {}} />);
        await screen.findAllByTestId("agent-my-agents-entry");
        tiles = measureInBrowser(document.body.innerHTML);
    }, 90_000);

    afterEach(() => {});
    afterAll(() => cleanup());

    it("measures every tile", () => {
        expect(tiles).toHaveLength(rows.length);
    });

    it("the grid item owns the tile's border, so anything positioned against it is positioned against the visible tile", () => {
        for (const t of tiles) expect(t.liBorderTop, t.name).toBeGreaterThan(0);
    });

    it("the tiles really differ in content height, and tiles sharing a grid row are the same height", () => {
        // Two columns at the 1000 px fixture width: tiles 0+1 share a row, 2+3 share a row.
        expect(tiles[0].li.top).toBeCloseTo(tiles[1].li.top, 0);
        expect(tiles[2].li.top).toBeCloseTo(tiles[3].li.top, 0);
        expect(tiles[2].li.top).toBeGreaterThan(tiles[0].li.bottom - 1);
        expect(tiles[0].li.height).toBeCloseTo(tiles[1].li.height, 0);
        expect(tiles[2].li.height).toBeCloseTo(tiles[3].li.height, 0);
        // The two rows differ from each other, so the fixture exercises varying heights.
        expect(Math.abs(tiles[0].li.height - tiles[2].li.height)).toBeGreaterThan(10);
    });

    it("the chevron sits inside its own tile, the same distance from the bottom on every tile", () => {
        const gaps = tiles.map((t) => t.li.bottom - t.toggle.bottom);
        for (const [i, t] of tiles.entries()) {
            expect(t.toggle.top, t.name).toBeGreaterThanOrEqual(t.li.top);
            expect(t.toggle.right, t.name).toBeLessThanOrEqual(t.li.right);
            expect(gaps[i], t.name).toBeGreaterThanOrEqual(0);
            expect(gaps[i], t.name).toBeLessThanOrEqual(24);
            expect(gaps[i], t.name).toBeCloseTo(gaps[0], 0);
        }
    });

    it("the chevron shares a row with the timestamps instead of floating over padding", () => {
        for (const t of tiles) {
            if (!t.stamps) continue;
            const chevronMid = (t.toggle.top + t.toggle.bottom) / 2;
            const stampsMid = (t.stamps.top + t.stamps.bottom) / 2;
            expect(Math.abs(chevronMid - stampsMid), t.name).toBeLessThanOrEqual(2);
            expect(t.stamps.right, t.name).toBeLessThanOrEqual(t.toggle.left + 1);
        }
    });

    it("a click anywhere on the tile, timestamps included, lands on the open button; the chevron stays its own control", () => {
        for (const t of tiles) {
            expect(t.hitCentre, t.name).toBe("entry");
            if (t.hitStamps !== null) expect(t.hitStamps, t.name).toBe("entry");
            expect(t.hitToggle, t.name).toBe("toggle");
        }
    });
});
