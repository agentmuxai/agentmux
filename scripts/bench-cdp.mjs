// bench-cdp.mjs — Measure scroll FPS and input latency via CDP
// Usage: node bench-cdp.mjs [cdp_port]
import { WebSocket } from "ws";
import http from "http";

const CDP_PORT = parseInt(process.argv[2] || "9222");
let msgId = 0;

async function getWsUrl() {
    return new Promise((resolve, reject) => {
        http.get(`http://localhost:${CDP_PORT}/json`, (res) => {
            let data = "";
            res.on("data", (chunk) => (data += chunk));
            res.on("end", () => {
                const targets = JSON.parse(data);
                const page = targets.find((t) => t.type === "page");
                if (!page) reject(new Error("No page target found"));
                resolve(page.webSocketDebuggerUrl);
            });
        }).on("error", reject);
    });
}

function createCDP(ws) {
    const pending = new Map();
    ws.on("message", (raw) => {
        const msg = JSON.parse(raw.toString());
        if (msg.id && pending.has(msg.id)) {
            pending.get(msg.id)(msg);
            pending.delete(msg.id);
        }
    });
    return {
        send(method, params = {}) {
            return new Promise((resolve) => {
                const id = ++msgId;
                pending.set(id, resolve);
                ws.send(JSON.stringify({ id, method, params }));
            });
        },
    };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function sendKey(cdp, key, code = 0) {
    if (key === "\n" || key === "Enter") {
        await cdp.send("Input.dispatchKeyEvent", {
            type: "rawKeyDown", key: "Enter", code: "Enter",
            windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13,
        });
        await cdp.send("Input.dispatchKeyEvent", {
            type: "char", key: "Enter", text: "\r",
        });
        await cdp.send("Input.dispatchKeyEvent", {
            type: "keyUp", key: "Enter", code: "Enter",
            windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13,
        });
    } else {
        const vk = key.toUpperCase().charCodeAt(0);
        await cdp.send("Input.dispatchKeyEvent", {
            type: "rawKeyDown", key, code: `Key${key.toUpperCase()}`,
            windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, text: key,
        });
        await cdp.send("Input.dispatchKeyEvent", {
            type: "char", key, text: key, unmodifiedText: key,
        });
        await cdp.send("Input.dispatchKeyEvent", {
            type: "keyUp", key, code: `Key${key.toUpperCase()}`,
            windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk,
        });
    }
}

async function sendString(cdp, str) {
    for (const ch of str) {
        if (ch === " ") {
            await cdp.send("Input.dispatchKeyEvent", {
                type: "rawKeyDown", key: " ", code: "Space",
                windowsVirtualKeyCode: 32, nativeVirtualKeyCode: 32, text: " ",
            });
            await cdp.send("Input.dispatchKeyEvent", { type: "char", text: " " });
            await cdp.send("Input.dispatchKeyEvent", {
                type: "keyUp", key: " ", code: "Space",
                windowsVirtualKeyCode: 32, nativeVirtualKeyCode: 32,
            });
        } else if (ch >= "0" && ch <= "9") {
            const vk = ch.charCodeAt(0);
            await cdp.send("Input.dispatchKeyEvent", {
                type: "rawKeyDown", key: ch, code: `Digit${ch}`,
                windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, text: ch,
            });
            await cdp.send("Input.dispatchKeyEvent", { type: "char", text: ch });
            await cdp.send("Input.dispatchKeyEvent", {
                type: "keyUp", key: ch, code: `Digit${ch}`,
                windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk,
            });
        } else if (ch === "\n") {
            await sendKey(cdp, "Enter");
        } else {
            await sendKey(cdp, ch);
        }
    }
}

async function main() {
    console.log(`Connecting to CDP on port ${CDP_PORT}...`);
    const wsUrl = await getWsUrl();
    const ws = new WebSocket(wsUrl);
    await new Promise((r) => ws.on("open", r));
    const cdp = createCDP(ws);

    await cdp.send("Runtime.enable");
    await cdp.send("Performance.enable");

    // ─── BASELINE METRICS ─────────────────────────
    const evalJS = async (expr) => {
        const r = await cdp.send("Runtime.evaluate", {
            expression: expr, returnByValue: true, awaitPromise: true,
        });
        return r.result?.result?.value;
    };

    const jsHeap = await evalJS(
        "performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1024 / 1024) : 'N/A'"
    );
    console.log(`\n=== BASELINE ===`);
    console.log(`  JS Heap: ${jsHeap} MB`);

    // ─── INPUT LATENCY ────────────────────────────
    console.log(`\n=== INPUT LATENCY (50 keystrokes) ===`);

    // Inject rAF frame counter
    await evalJS(`
        window.__benchFrameCount = 0;
        window.__benchRafRunning = true;
        (function loop() {
            window.__benchFrameCount++;
            if (window.__benchRafRunning) requestAnimationFrame(loop);
        })();
    `);

    const latencies = [];
    for (let i = 0; i < 50; i++) {
        const char = String.fromCharCode(97 + (i % 26));
        const t0 = performance.now();

        await sendKey(cdp, char);

        // Wait for 2 rAF cycles (input → render)
        await evalJS(
            "new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))"
        );

        latencies.push(performance.now() - t0);
    }

    // Send Enter to clear the junk we typed
    await sendKey(cdp, "Enter");
    await sleep(300);

    latencies.sort((a, b) => a - b);
    const p = (pct) => latencies[Math.floor(latencies.length * pct)].toFixed(1);
    console.log(`  P50:  ${p(0.5)} ms`);
    console.log(`  P95:  ${p(0.95)} ms`);
    console.log(`  P99:  ${p(0.99)} ms`);
    console.log(`  Min:  ${latencies[0].toFixed(1)} ms`);
    console.log(`  Max:  ${latencies[latencies.length - 1].toFixed(1)} ms`);
    console.log(`  Mean: ${(latencies.reduce((a, b) => a + b, 0) / latencies.length).toFixed(1)} ms`);

    // ─── SCROLL FPS ───────────────────────────────
    console.log(`\n=== SCROLL FPS (seq 1 50000) ===`);

    // Reset frame counter
    await evalJS("window.__benchFrameCount = 0");
    const scrollT0 = performance.now();

    // Type the command
    await sendString(cdp, "seq 1 50000\n");

    // Wait for it to finish
    console.log("  Waiting for output...");
    await sleep(10000);

    const scrollT1 = performance.now();
    const frameCount = await evalJS("window.__benchFrameCount");
    const scrollSec = (scrollT1 - scrollT0) / 1000;
    const fps = (frameCount / scrollSec).toFixed(1);

    console.log(`  Duration: ${scrollSec.toFixed(1)} s`);
    console.log(`  Frames: ${frameCount}`);
    console.log(`  FPS: ${fps}`);

    // ─── POST-SCROLL MEMORY ───────────────────────
    const postHeap = await evalJS(
        "performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1024 / 1024) : 'N/A'"
    );
    console.log(`\n=== POST-SCROLL ===`);
    console.log(`  JS Heap: ${postHeap} MB`);

    // Cleanup
    await evalJS("window.__benchRafRunning = false");
    ws.close();
    console.log("\nDone.");
}

main().catch((e) => {
    console.error(e);
    process.exit(1);
});
