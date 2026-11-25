// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tasks on an Atlas node, measured in a real browser (atlas-node-tasks-view.tsx, docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md
 * §6.3): the node is as tall as the layout says (`nodeHeight`), its name and state keep the height every node has with the
 * tasks under them, a task is one line cut with an ellipsis before its pull request's link, nothing runs out of the node
 * sideways, and a press on a link can start no text selection (`user-select: none` from the map, which is what the
 * pane's selection guard reads: pane-select.ts `cannotSelect`). Compiles the real `atlas.scss` and the kit's `ui.scss`; at
 * two pane zooms. It skips, rather than fails, on a machine with no usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";
import { NODE_H, nodeHeight, TASK_LINE_H, TASKS_PAD_BOTTOM } from "./atlas-layout";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = [join(__dirname, "atlas.scss"), join(__dirname, "../../element/ui/ui.scss")].map((f) => sass.compile(f).css).join("\n");

const TOKENS = `:root{--main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--border-color:#333;--main-bg-color:#222;
--hover-bg-color:#2a2a2a;--error-text-color:#e55;--success-text-color:#5c5;--warning-text-color:#ec5;--highlight-bg-color:#333;
--kanban-stage-progress:#4a90d9;--kanban-stage-needs:#e8a33d;--kanban-stage-review:#b583e6;--kanban-stage-backlog:#888;
--font-sans:Arial,sans-serif;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--leading-tight:1.2;
--space-1:4px;--space-2:8px;--space-3:12px;--radius-sm:3px;--radius-full:999px;--duration-quick:0s;
--ui-height:28px;--ui-pad-x:12px;--ui-pad-y:4px;--ui-gap:6px;--ui-font-size:13px;--ui-icon-size:12px;--ui-font-weight:500}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}.pane{margin:6px 0;position:relative;height:200px}`;

const LONG = "Connectors: deleting an account asks first, in the card, not in a window";

interface Case {
    id: string;
    name: string;
    host: string;
    count?: number;
    tasks: { num: number; title: string; pr?: string; held?: boolean }[];
    more?: number;
}

const CASES: Case[] = [
    { id: "one", name: "Tansy", host: "narko", tasks: [{ num: 48661, title: "Atlas: tasks on nodes" }] },
    {
        id: "full",
        name: "Opaz",
        host: "charlie",
        tasks: [
            { num: 48652, title: LONG, pr: "#8301", held: true },
            { num: 48607, title: "History search time limit", pr: "#8273", held: true },
            { num: 48362, title: "Recovery row after reload", held: false },
        ],
        more: 2,
    },
    { id: "long-name", name: "An-agent-with-a-long-name", host: "starpower", tasks: [{ num: 12, title: LONG, pr: "#9" }] },
    { id: "none", name: "Idle", host: "narko", tasks: [] },
    { id: "count", name: "Busy", host: "narko", count: 3, tasks: [] },
];

function nodeHtml(c: Case, left: number): string {
    const lines = c.tasks.length + (c.more ? 1 : 0);
    const h = nodeHeight(lines);
    const tasks = c.tasks
        .map(
            (t) =>
                `<li class="atlas-node__task-line${t.held === false ? " atlas-node__task-line--waiting" : ""}" style="--atlas-task-stage:var(--kanban-stage-progress)">` +
                `<span class="atlas-node__task-mark"></span>` +
                `<button type="button" class="ui-button ui-tone-quiet ui-button--link ui-button--opens atlas-node__task">#${t.num} ${t.title}</button>` +
                (t.pr ? `<button type="button" class="ui-button ui-tone-quiet ui-button--link ui-button--opens atlas-node__task-pr">${t.pr}</button>` : "") +
                `</li>`
        )
        .join("");
    const more = c.more ? `<li class="atlas-node__task-line atlas-node__more">+${c.more} more</li>` : "";
    return (
        `<div class="atlas-node atlas-node--working${lines > 0 ? " atlas-node--tasks" : ""}" id="${c.id}" style="left:${left}px;top:6px;width:140px;height:${h}px">` +
        `<span class="atlas-node__head"><span class="atlas-node__top"><span class="atlas-node__dot"></span><span class="atlas-node__name">${c.name}</span>` +
        (c.count ? `<span class="atlas-node__count"><i class="fa-solid"></i>${c.count}</span>` : "") +
        `<span class="atlas-node__host">${c.host}</span></span><span class="atlas-node__sub">Working</span></span>` +
        (lines > 0 ? `<ul class="atlas-node__tasks">${tasks}${more}</ul>` : "") +
        `</div>`
    );
}

const ZOOMS = [1, 1.5];

const MEASURE = `
const result = {};
for (const pane of document.querySelectorAll('.pane')) {
  const z = Number(pane.dataset.zoom);
  const unit = (r) => r / z;
  for (const node of pane.querySelectorAll('.atlas-node')) {
    const rect = node.getBoundingClientRect();
    const head = node.querySelector('.atlas-node__head').getBoundingClientRect();
    const tasks = node.querySelector('.atlas-node__tasks');
    const lines = [...node.querySelectorAll('.atlas-node__task-line')].map((l) => l.getBoundingClientRect());
    const link = node.querySelector('.atlas-node__task');
    const pr = node.querySelector('.atlas-node__task-pr');
    const name = node.querySelector('.atlas-node__name').getBoundingClientRect();
    const host = node.querySelector('.atlas-node__host').getBoundingClientRect();
    result[node.id + '@' + z] = {
      height: unit(rect.height),
      headHeight: unit(head.height),
      headBottom: unit(head.bottom - rect.top),
      tasksTop: tasks ? unit(tasks.getBoundingClientRect().top - rect.top) : null,
      lineHeights: lines.map((l) => unit(l.height)),
      lineTops: lines.map((l) => unit(l.top - rect.top)),
      overflowX: node.scrollWidth - node.clientWidth,
      linkCut: link ? link.scrollWidth > link.clientWidth + 1 : null,
      titleStart: link ? (() => { const r = document.createRange(); r.selectNodeContents(link); return unit(r.getBoundingClientRect().left - link.getBoundingClientRect().left); })() : null,
      linkDisplay: link ? getComputedStyle(link).display : null,
      linkRight: link ? unit(link.getBoundingClientRect().right - rect.left) : null,
      linkHeight: link ? unit(link.getBoundingClientRect().height) : null,
      prLeft: pr ? unit(pr.getBoundingClientRect().left - rect.left) : null,
      prRight: pr ? unit(pr.getBoundingClientRect().right - rect.left) : null,
      prWidthWhole: pr ? pr.scrollWidth <= pr.clientWidth + 1 : null,
      userSelect: link ? getComputedStyle(link).userSelect : null,
      nameHostOverlap: unit(Math.max(0, name.right - host.left)),
      nodeWidth: unit(rect.width),
    };
  }
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    height: number;
    headHeight: number;
    headBottom: number;
    tasksTop: number | null;
    lineHeights: number[];
    lineTops: number[];
    overflowX: number;
    linkCut: boolean | null;
    titleStart: number | null;
    linkDisplay: string | null;
    linkRight: number | null;
    linkHeight: number | null;
    prLeft: number | null;
    prRight: number | null;
    prWidthWhole: boolean | null;
    userSelect: string | null;
    nameHostOverlap: number;
    nodeWidth: number;
}

let measured: Record<string, Measured> = {};
const at = (id: string, z: number) => measured[`${id}@${z}`];

beforeAll(async () => {
    if (!BROWSER) return;
    const panes = ZOOMS.map(
        (z) =>
            `<div class="pane" data-zoom="${z}" style="zoom:${z}"><div class="atlas-viewport" style="position:absolute;inset:0">` +
            `<div class="atlas-world" style="position:relative">${CASES.map((c, i) => nodeHtml(c, 6 + i * 160)).join("")}</div></div></div>`
    ).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${panes}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 1000, height: 900 }
    );
}, 90_000);

describe.skipIf(!BROWSER)("Atlas node tasks", () => {
    it("is as tall as the layout says, and no taller", () => {
        for (const z of ZOOMS) {
            expect(at("one", z).height).toBeCloseTo(nodeHeight(1), 0);
            expect(at("full", z).height).toBeCloseTo(nodeHeight(4), 0);
            expect(at("none", z).height).toBeCloseTo(NODE_H, 0);
        }
    });

    it("keeps the name and the state the height every node has, and the tasks start under them", () => {
        for (const z of ZOOMS) {
            for (const id of ["one", "full", "long-name"]) {
                const m = at(id, z);
                expect(m.headHeight, `${id} head at zoom ${z}`).toBeCloseTo(NODE_H - 2, 0);
                expect(m.tasksTop!, `${id} tasks at zoom ${z}`).toBeGreaterThanOrEqual(m.headBottom - 1);
            }
        }
    });

    it("is a line of its own height for each task, one under another", () => {
        for (const z of ZOOMS) {
            const m = at("full", z);
            expect(m.lineHeights).toHaveLength(4);
            for (const h of m.lineHeights) expect(h, `a line at zoom ${z}`).toBeCloseTo(TASK_LINE_H, 0);
            m.lineTops.slice(1).forEach((top, i) => expect(top - m.lineTops[i], `line ${i + 1} at zoom ${z}`).toBeCloseTo(TASK_LINE_H, 0));
            expect(m.lineTops[0] + TASK_LINE_H * 4, `bottom at zoom ${z}`).toBeLessThanOrEqual(m.height + 2);
            // A little room under the last line: its letters do not touch the node's bottom border.
            expect(m.height - (m.lineTops[0] + TASK_LINE_H * 4), `room under the last line at zoom ${z}`).toBeGreaterThanOrEqual(TASKS_PAD_BOTTOM - 1);
        }
    });

    it("cuts a long title with an ellipsis and keeps the pull request's link whole beside it, inside the node", () => {
        for (const z of ZOOMS) {
            const m = at("full", z);
            expect(m.linkCut, `title cut at zoom ${z}`).toBe(true);
            // The start of the title is what shows, cut at its end with an ellipsis (a flex box centres what overflows and draws no ellipsis).
            expect(m.titleStart!, `title starts at the link's left edge at zoom ${z}`).toBeGreaterThanOrEqual(-0.5);
            expect(m.linkDisplay, `link is a block of text at zoom ${z}`).not.toMatch(/flex/);
            expect(m.prWidthWhole).toBe(true);
            expect(m.linkRight!, `title ends before the pull request at zoom ${z}`).toBeLessThanOrEqual(m.prLeft! + 1);
            expect(m.prRight!, `pull request inside the node at zoom ${z}`).toBeLessThanOrEqual(m.nodeWidth);
            expect(m.linkHeight!, `link height at zoom ${z}`).toBeLessThanOrEqual(TASK_LINE_H + 1);
        }
    });

    it("never runs out of the node sideways, with a long name, a long title or the count", () => {
        for (const z of ZOOMS) for (const c of CASES) expect(at(c.id, z).overflowX, `${c.id} at zoom ${z}`).toBeLessThanOrEqual(1);
    });

    it("keeps the name clear of the host's name with the count between them", () => {
        for (const z of ZOOMS) expect(at("count", z).nameHostOverlap, `at zoom ${z}`).toBeLessThanOrEqual(1);
    });

    it("lets a press on a link start no text selection (user-select: none, from the map)", () => {
        for (const z of ZOOMS) for (const id of ["one", "full", "long-name"]) expect(at(id, z).userSelect, `${id} at zoom ${z}`).toBe("none");
    });
});
