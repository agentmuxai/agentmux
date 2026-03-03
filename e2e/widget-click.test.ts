/**
 * Widget click e2e test for Tauri app (WebView2 via CDP).
 *
 * Restarts the running agentmux.exe with WebView2 remote debugging enabled,
 * connects via CDP, and clicks widget buttons.
 */
import { test, expect, chromium, Browser, Page } from "playwright/test";
import { execSync } from "child_process";

const APP_PATH = "C:\\Systems\\agentmux\\target\\debug\\agentmux.exe";
const CDP_PORT = 9333;

let browser: Browser;
let page: Page;

async function tryGetReadyPage(): Promise<Page | null> {
    let b: Browser;
    try {
        b = await chromium.connectOverCDP(`http://localhost:${CDP_PORT}`, { timeout: 3000 });
    } catch {
        return null;
    }
    for (const ctx of b.contexts()) {
        for (const p of ctx.pages()) {
            try {
                const url = p.url();
                if (!url.includes("tauri.localhost")) continue;
                const mainCount = await p.evaluate(
                    () => document.getElementById("main")?.children.length ?? 0
                );
                console.log(`  url=${url} #main.children=${mainCount}`);
                if (mainCount > 0) {
                    browser = b;
                    return p;
                }
            } catch {}
        }
    }
    await b.close().catch(() => {});
    return null;
}

test.beforeAll(async () => {
    // Kill only the Tauri UI process — leave agentmuxsrv-rs running for reuse
    try {
        execSync("taskkill /F /IM agentmux.exe 2>nul", { shell: "cmd.exe" });
        await new Promise((r) => setTimeout(r, 2000));
    } catch {}

    // Relaunch with CDP port enabled
    execSync(
        `powershell.exe -Command "` +
        `$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=${CDP_PORT}'; ` +
        `Start-Process '${APP_PATH}'"`,
        { shell: "cmd.exe" }
    );

    console.log("App relaunched with CDP. Waiting for ready state (up to 60s)...");
    const deadline = Date.now() + 60000;
    while (Date.now() < deadline) {
        const p = await tryGetReadyPage();
        if (p) {
            page = p;
            console.log("App ready!");
            return;
        }
        await new Promise((r) => setTimeout(r, 2000));
    }
    throw new Error("App did not become ready within 60s");
}, 90000);

test.afterAll(async () => {
    await browser?.close().catch(() => {});
    // Restart app cleanly without CDP flag
    try {
        execSync("taskkill /F /IM agentmux.exe 2>nul", { shell: "cmd.exe" });
        await new Promise((r) => setTimeout(r, 500));
        execSync(`start "" "${APP_PATH}"`, { shell: "cmd.exe" });
    } catch {}
});

test.describe("Widget buttons", () => {
    test("action-widgets bar is visible", async () => {
        await page.screenshot({ path: "e2e/screenshots/widget-01-initial.png" });

        const widgetBar = page.locator('[data-testid="action-widgets"]');
        await expect(widgetBar).toBeVisible({ timeout: 10000 });
        console.log("Widget bar found!");

        const count = await widgetBar.locator("> *").count();
        console.log(`Widget count: ${count}`);
        await page.screenshot({ path: "e2e/screenshots/widget-02-bar.png" });
    });

    test("clicking terminal widget creates a new block", async () => {
        const blocksBefore = await page.locator("[data-blockid]").count();
        console.log("Blocks before:", blocksBefore);

        const widgetBar = page.locator('[data-testid="action-widgets"]');
        await expect(widgetBar).toBeVisible({ timeout: 10000 });

        const termWidget = widgetBar.locator("text=terminal").first();
        await expect(termWidget).toBeVisible({ timeout: 5000 });

        // Capture any errors during click
        const errors: string[] = [];
        page.on("console", (msg) => { if (msg.type() === "error") errors.push(msg.text()); });
        page.on("pageerror", (err) => errors.push(err.message));

        console.log("Clicking terminal widget...");
        await termWidget.click();
        await new Promise((r) => setTimeout(r, 2000));

        await page.screenshot({ path: "e2e/screenshots/widget-03-after-click.png" });

        const blocksAfter = await page.locator("[data-blockid]").count();
        console.log("Blocks after:", blocksAfter);
        if (errors.length) console.log("Errors:", errors);

        expect(blocksAfter).toBeGreaterThan(blocksBefore);
    });

    test("all widgets listed", async () => {
        const widgetBar = page.locator('[data-testid="action-widgets"]');
        await expect(widgetBar).toBeVisible({ timeout: 5000 });
        const widgets = widgetBar.locator("> *");
        const count = await widgets.count();
        for (let i = 0; i < count; i++) {
            const text = await widgets.nth(i).textContent();
            console.log(`Widget ${i}: "${text?.trim()}"`);
        }
        expect(count).toBeGreaterThan(0);
    });
});
