// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
import { connectBrowser, connectPage } from "../../ui-screenshots/lib/cdp-client.mjs";
const browser = await connectBrowser(9223);
const info = await browser.send("SystemInfo.getInfo");
const gpu = info.gpu;
console.log("GPU devices:", JSON.stringify(gpu.devices.map((d) => ({ vendor: d.vendorString, device: d.deviceString, driver: d.driverVendor })), null, 0));
console.log("GL:", JSON.stringify({ vendor: gpu.auxAttributes?.glVendor, renderer: gpu.auxAttributes?.glRenderer, version: gpu.auxAttributes?.glVersion, displayType: gpu.auxAttributes?.glImplementationParts }, null, 0));
console.log("featureStatus:", JSON.stringify(gpu.featureStatus));
console.log("driverBugWorkarounds:", JSON.stringify(gpu.driverBugWorkarounds?.slice(0, 12)));
console.log("commandLine:", (info.commandLine || "").split(" --").filter((s) => /gpu|angle|gl|raster|feature|compos|frame|vsync|backend|metal|graphite|skia|canvas|zero|ui-/.test(s)).join(" --"));
const { session: page } = await connectPage(9223, "window_transparent");
const r = await page.send("Runtime.evaluate", { awaitPromise: true, returnByValue: true, expression: `(async () => {
  const gaps = []; let last = 0, n = 0;
  await new Promise((res) => { const f = (t) => { if (last) gaps.push(t - last); last = t; if (++n < 180) requestAnimationFrame(f); else res(); }; requestAnimationFrame(f); });
  gaps.sort((a, b) => a - b);
  const c = document.createElement("canvas"); const gl = c.getContext("webgl2"); const ext = gl && gl.getExtension("WEBGL_debug_renderer_info");
  return {
    dpr: devicePixelRatio, inner: [innerWidth, innerHeight], screen: [screen.width, screen.height, screen.colorDepth],
    rafIdleP50: +gaps[Math.floor(gaps.length / 2)].toFixed(2), rafIdleP90: +gaps[Math.floor(gaps.length * 0.9)].toFixed(2),
    webgl2: !!gl, unmaskedRenderer: ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : null,
    bodyTransform: getComputedStyle(document.body).transform, keepLaidOut: document.querySelectorAll('[data-window-tab], .tab-content').length,
    mediaQueries: matchMedia("(prefers-reduced-motion: reduce)").matches
  };
})()` });
console.log("page:", JSON.stringify(r.result.value));
process.exit(0);
