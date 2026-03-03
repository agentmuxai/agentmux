import { test, expect, _electron as electron, ElectronApplication, Page } from "@playwright/test";
import path from "path";
import { fileURLToPath } from "url";
import fs from "fs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

let electronApp: ElectronApplication;
let window: Page;

test.beforeAll(async () => {
    // Launch the Electron app from the production build
    const appPath = path.join(__dirname, "..", "make", "win-unpacked", "AgentMux.exe");

    console.log("Launching AgentMux from:", appPath);

    electronApp = await electron.launch({
        executablePath: appPath,
        args: [],
        timeout: 60000,
    });

    console.log("electron.launch() completed, waiting for first window...");

    // Wait for the first window to open
    window = await electronApp.firstWindow();
    console.log("Got first window, waiting for DOM...");

    await window.waitForLoadState("domcontentloaded");
    console.log("DOM loaded, waiting for app to initialize...");

    // Give the app time to fully initialize
    await window.waitForTimeout(5000);
    console.log("App initialized, ready to test");
});

test.afterAll(async () => {
    if (electronApp) {
        await electronApp.close();
    }
});

test.describe("Close Button Tests", () => {
    test("should only close one pane when clicking close button", async () => {
        // Take a screenshot of initial state
        await window.screenshot({ path: "e2e/screenshots/01-initial.png" });

        // Count the initial number of terminal blocks/panes
        // Look for elements with data-blockid attribute (terminal blocks)
        const initialBlocks = await window.locator('[data-blockid]').all();
        const initialCount = initialBlocks.length;
        console.log(`Initial block count: ${initialCount}`);

        // If only one block, we need to split first
        if (initialCount <= 1) {
            console.log("Only one block, need to create more for testing");

            // Try to find and click the split button or use keyboard shortcut
            // First let's see what's on the page
            const pageContent = await window.content();
            console.log("Page has blocks:", initialCount);

            // Try Ctrl+Shift+D to split (common shortcut)
            await window.keyboard.press("Control+Shift+d");
            await window.waitForTimeout(1000);
            await window.screenshot({ path: "e2e/screenshots/02-after-split-attempt.png" });
        }

        // Re-count blocks after potential split
        const blocksAfterSplit = await window.locator('[data-blockid]').all();
        const countAfterSplit = blocksAfterSplit.length;
        console.log(`Block count after split attempt: ${countAfterSplit}`);

        // Take screenshot
        await window.screenshot({ path: "e2e/screenshots/03-before-close.png" });

        // Find the close button (X) - it should be in the block header
        // Looking for common close button patterns
        const closeButtons = await window.locator('.block-frame-close, .close-btn, [class*="close"], button:has-text("×")').all();
        console.log(`Found ${closeButtons.length} close buttons`);

        if (closeButtons.length > 0) {
            // Click the first close button
            await closeButtons[0].click();
            await window.waitForTimeout(1000);

            // Take screenshot after close
            await window.screenshot({ path: "e2e/screenshots/04-after-close.png" });

            // Count blocks after close
            const blocksAfterClose = await window.locator('[data-blockid]').all();
            const countAfterClose = blocksAfterClose.length;
            console.log(`Block count after close: ${countAfterClose}`);

            // THE BUG: All blocks close instead of just one
            // Expected: countAfterClose === countAfterSplit - 1
            // Bug behavior: countAfterClose === 0 (all closed)

            if (countAfterClose === 0 && countAfterSplit > 1) {
                console.error("BUG DETECTED: All panes closed instead of just one!");
            }

            expect(countAfterClose).toBe(countAfterSplit - 1);
        } else {
            console.log("No close buttons found - need to investigate selectors");
            // Dump HTML structure for debugging
            const html = await window.content();
            fs.writeFileSync("e2e/screenshots/page-dump.html", html);
        }
    });
});
