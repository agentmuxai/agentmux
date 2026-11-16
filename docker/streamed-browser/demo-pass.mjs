#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One scripted pass through the streamed Browser pane's web UI, recorded as a video. Headless, synthetic input only,
// demo pages only. docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md; README.md in this folder.
//
// Run on the host that runs the demo container, with Playwright and its Chromium available:
//   node demo-pass.mjs --out <dir> [--container streamed-demo] [--base http://127.0.0.1:8190] [--pages http://127.0.0.1:8200]
//
// `--base` and `--pages` must reach the container's own loopback (run this in a container that shares the demo's network
// namespace: `docker run --network container:streamed-demo ...`), because srv accepts only the address 127.0.0.1:8190.
// The sign-in is the real one: this runs `agentmux-srv open --print` in the container (through `script`, for a terminal),
// reads the page's check phrase and the terminal's, and answers y only when they are the same.
//
// Each step is checked against what the demo page itself reported (`/log` on the pages server), not against a picture.
// Prints one JSON line per step, then a summary; the video is in --out.

import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
    const i = args.indexOf(`--${name}`);
    return i >= 0 ? args[i + 1] : fallback;
};
const out = path.resolve(opt("out", "demo-pass-out"));
const container = opt("container", "streamed-demo");
const base = opt("base", "http://127.0.0.1:8190");
const pages = opt("pages", "http://127.0.0.1:8200");
fs.mkdirSync(out, { recursive: true });

const steps = [];
function record(step, ok, note) {
    const row = { step, ok, note };
    steps.push(row);
    console.log(JSON.stringify(row));
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The page's own list of what it received. */
async function pageLog() {
    const resp = await fetch(`${pages}/log`);
    return resp.json();
}
async function waitForEvent(match, since, ms = 8000) {
    const end = Date.now() + ms;
    while (Date.now() < end) {
        const hit = (await pageLog()).find((e) => e.t >= since && match(e));
        if (hit) return hit;
        await sleep(200);
    }
    return null;
}

/** Start `agentmux-srv open --print` in the container, with a terminal; hands back the sign-in line and the way to answer. */
function startOpen() {
    const child = spawn("script", ["-qec", `docker exec -it ${container} agentmux-srv open --print`, "/dev/null"], { stdio: ["pipe", "pipe", "pipe"] });
    let text = "";
    const waiters = [];
    child.stdout.on("data", (d) => {
        text += d.toString();
        for (const w of waiters) w();
    });
    const until = (re, ms = 20000) =>
        new Promise((resolve, reject) => {
            const t = setTimeout(() => reject(new Error(`the terminal never showed ${re}`)), ms);
            const check = () => {
                const m = text.match(re);
                if (m) {
                    clearTimeout(t);
                    resolve(m);
                }
            };
            waiters.push(check);
            check();
        });
    return { child, until, text: () => text };
}

async function signIn(context, page) {
    const open = startOpen();
    const line = (await open.until(/http:\/\/127\.0\.0\.1:\d+\/auth\/enter#\S+/))[0];
    await page.goto(line);
    // The page shows its phrase; the terminal then asks about the phrase it saw.
    const asked = (await open.until(/entered with "([^"]+)"/))[1];
    await page.waitForFunction((phrase) => document.body.innerText.includes(phrase), asked, { timeout: 15000 }).catch(() => null);
    const shown = await page.evaluate(() => document.body.innerText);
    if (!shown.includes(asked)) {
        open.child.stdin.write("n\n");
        throw new Error("the page did not show the phrase the terminal named");
    }
    open.child.stdin.write("y\n");
    await page.waitForURL(`${base}/`, { timeout: 30000 });
    record("sign in", true, `phrase "${asked}" matched on both sides`);
    open.child.kill();
}

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1280, height: 720 }, recordVideo: { dir: out, size: { width: 1280, height: 720 } } });
const page = await context.newPage();
let failed = null;
try {
    await signIn(context, page);

    // Click Browser, load the first page.
    await page.getByText("Browser", { exact: true }).first().click({ timeout: 30000 });
    const address = page.getByLabel("Address");
    await address.waitFor({ timeout: 30000 });
    const stage = page.locator(".streamed-stage");
    const canvas = page.locator(".streamed-canvas");
    let since = Date.now();
    await address.fill(`${pages}/motion.html`);
    await address.press("Enter");
    const loaded = await waitForEvent((e) => e.k === "load" && e.v.startsWith("/motion.html"), since, 20000);
    record("load /motion.html", !!loaded, loaded ? `the page loaded at ${loaded.v}` : "the page never reported loading");
    await sleep(5000);
    const frameSize = await canvas.evaluate((c) => `${c.width}x${c.height}`);
    record("frames arrive", frameSize !== "300x150", `the canvas holds frames of ${frameSize}`);

    const at = async (x, y) => {
        const box = await canvas.boundingBox();
        return { x: box.x + x, y: box.y + y };
    };

    // The form page: click a field, type, paste.
    await address.fill(`${pages}/form.html`);
    await address.press("Enter");
    await waitForEvent((e) => e.k === "load" && e.v.startsWith("/form.html"), since, 20000);
    since = Date.now();
    let p = await at(100, 96);
    await page.mouse.click(p.x, p.y);
    const click = await waitForEvent((e) => e.k === "click", since);
    record("click a field", !!click && click.v === "100,96", click ? `the page saw a click at ${click.v}` : "the page saw no click");
    since = Date.now();
    await page.keyboard.type("hello", { delay: 60 });
    const typed = await waitForEvent((e) => e.k === "input" && e.v === "hello", since);
    record("type", !!typed, typed ? "the field holds hello" : "the page never saw the text");
    p = await at(100, 160);
    await page.mouse.click(p.x, p.y);
    since = Date.now();
    await stage.evaluate((el) => {
        const data = new DataTransfer();
        data.setData("text/plain", "pasted text");
        el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
    });
    const pasted = await waitForEvent((e) => e.k === "input" && e.v === "pasted text", since);
    record("paste", !!pasted, pasted ? "the text area holds the pasted text (a synthetic paste event, not the system clipboard)" : "the page never saw the paste");

    // The long page: the wheel.
    await address.fill(`${pages}/scroll.html`);
    await address.press("Enter");
    await waitForEvent((e) => e.k === "load" && e.v.startsWith("/scroll.html"), since, 20000);
    since = Date.now();
    p = await at(400, 300);
    await page.mouse.move(p.x, p.y);
    await page.mouse.wheel(0, 600);
    const scrolled = await waitForEvent((e) => e.k === "scroll" && Number(e.v) > 0, since);
    record("wheel", !!scrolled, scrolled ? `the page scrolled to ${scrolled.v}` : "the page never scrolled");

    // The keys page: a few keys, with the stage focused.
    await address.fill(`${pages}/keys.html`);
    await address.press("Enter");
    await waitForEvent((e) => e.k === "load" && e.v.startsWith("/keys.html"), since, 20000);
    since = Date.now();
    p = await at(300, 300);
    await page.mouse.click(p.x, p.y);
    for (const k of ["a", "b", "Enter", "ArrowLeft"]) await page.keyboard.press(k);
    await sleep(800);
    const keys = (await pageLog()).filter((e) => e.k === "key" && e.t >= since).map((e) => e.v);
    const want = ["a", "b", "Enter", "ArrowLeft"];
    record("keys", want.every((k) => keys.includes(k)), `the page saw: ${keys.join(" ")}`);

    // One resize of the pane: the window gets smaller, the page follows.
    since = Date.now();
    await page.setViewportSize({ width: 1000, height: 640 });
    const box = await stage.boundingBox();
    const resized = await waitForEvent((e) => e.k === "resize" && e.v === `${Math.floor(box.width)}x${Math.floor(box.height)}`, since, 10000);
    record("resize", !!resized, resized ? `the page followed the pane to ${resized.v}` : `the pane is ${Math.floor(box.width)}x${Math.floor(box.height)} and the page never reported that size`);
    await sleep(2000);
} catch (e) {
    failed = e;
    record("the pass", false, String(e.message ?? e));
}
await context.close();
await browser.close();
const ok = steps.length > 0 && steps.every((s) => s.ok) && !failed;
console.log(JSON.stringify({ summary: { ok, steps: steps.length, failed: steps.filter((s) => !s.ok).map((s) => s.step) } }));
process.exit(ok ? 0 : 1);
