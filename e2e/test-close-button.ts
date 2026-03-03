/**
 * Close Button Bug Test Script
 *
 * This script tests the close button functionality by:
 * 1. Launching the production AgentMux build
 * 2. Creating 2 terminal panes
 * 3. Clicking the close button on one pane
 * 4. Verifying that only one pane closes (not all)
 */

import { _electron as electron, ElectronApplication } from "playwright";
import * as path from "path";
import * as fs from "fs";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const SCREENSHOT_DIR = path.join(__dirname, "screenshots");
const APP_PATH = path.join(__dirname, "..", "make", "win-unpacked", "AgentMux.exe");

async function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}

async function main() {
    console.log("=== Close Button Bug Test ===\n");

    // Ensure screenshots directory exists
    if (!fs.existsSync(SCREENSHOT_DIR)) {
        fs.mkdirSync(SCREENSHOT_DIR, { recursive: true });
    }

    console.log(`App path: ${APP_PATH}`);
    console.log(`App exists: ${fs.existsSync(APP_PATH)}`);

    if (!fs.existsSync(APP_PATH)) {
        console.error("ERROR: AgentMux.exe not found. Run 'task package' first.");
        process.exit(1);
    }

    let electronApp: ElectronApplication | null = null;
    let consoleErrors: string[] = [];

    try {
        console.log("\n1. Launching AgentMux...");
        electronApp = await electron.launch({
            executablePath: APP_PATH,
            timeout: 60000,
        });
        console.log("   App launched successfully");

        console.log("\n2. Waiting for main window...");
        const window = await electronApp.firstWindow();
        console.log("   Got main window");

        await window.waitForLoadState("domcontentloaded");
        console.log("   DOM content loaded");

        // Wait for app to fully initialize
        console.log("\n3. Waiting for app initialization...");
        await sleep(5000);

        // Listen for console errors
        window.on('console', msg => {
            if (msg.type() === 'error') {
                const text = msg.text();
                consoleErrors.push(text);
                console.log(`   [CONSOLE ERROR] ${text}`);
            }
        });

        // Listen for page errors (uncaught exceptions)
        window.on('pageerror', error => {
            console.log(`   [PAGE ERROR] ${error.message}`);
        });

        // Take initial screenshot
        await window.screenshot({ path: path.join(SCREENSHOT_DIR, "01-initial.png") });
        console.log("   Screenshot: 01-initial.png");

        // Create first terminal
        console.log("\n4. Creating first terminal...");
        const terminalWidget = window.locator('text=terminal').first();
        await terminalWidget.click();
        await sleep(3000);

        // Count close buttons to determine number of panes
        let closeButtonCount = await window.locator('.block-frame-default-close').count();
        console.log(`   Close buttons after first terminal: ${closeButtonCount}`);

        // Create second terminal
        console.log("\n5. Creating second terminal...");
        await terminalWidget.click();
        await sleep(3000);

        await window.screenshot({ path: path.join(SCREENSHOT_DIR, "02-two-terminals.png") });
        console.log("   Screenshot: 02-two-terminals.png");

        closeButtonCount = await window.locator('.block-frame-default-close').count();
        console.log(`   Close buttons after second terminal: ${closeButtonCount}`);

        if (closeButtonCount < 2) {
            console.log("\n   WARNING: Could not create 2 terminals. Test may not be accurate.");
        }

        const panesBeforeClose = closeButtonCount;

        // Click the first close button
        console.log("\n6. Clicking close button...");
        const closeBtn = window.locator('.block-frame-default-close').first();
        const closeBtnBox = await closeBtn.boundingBox();

        if (closeBtnBox) {
            console.log(`   Close button position: ${closeBtnBox.x}, ${closeBtnBox.y}`);
            await closeBtn.click();
            console.log("   Clicked close button");
        } else {
            // Try hovering first
            console.log("   Close button not visible, hovering over pane first...");
            const terminalHeader = window.locator('.block-frame-default-header').first();
            await terminalHeader.hover();
            await sleep(500);
            await closeBtn.click({ force: true });
            console.log("   Clicked close button (with force)");
        }

        await sleep(2000);
        await window.screenshot({ path: path.join(SCREENSHOT_DIR, "03-after-close.png") });
        console.log("   Screenshot: 03-after-close.png");

        const closeButtonCountAfter = await window.locator('.block-frame-default-close').count();
        const panesAfterClose = closeButtonCountAfter;
        console.log(`   Close buttons after close: ${closeButtonCountAfter}`);

        // Check if tab bar still exists
        const tabBarExists = await window.locator('.tab-bar').count() > 0;
        console.log(`   Tab bar exists: ${tabBarExists}`);

        // Check tile layout
        const tileLayoutExists = await window.locator('.tile-layout').count() > 0;
        console.log(`   Tile layout exists: ${tileLayoutExists}`);

        // Analyze result
        // Note: Each pane has 2 close button elements, so divide by 2 for actual pane count
        const actualPanesBefore = Math.floor(panesBeforeClose / 2);
        const actualPanesAfter = Math.floor(panesAfterClose / 2);

        console.log("\n=== RESULT ===");
        console.log(`Close buttons before: ${panesBeforeClose} (~${actualPanesBefore} panes)`);
        console.log(`Close buttons after: ${panesAfterClose} (~${actualPanesAfter} panes)`);
        console.log(`Console errors: ${consoleErrors.length}`);

        // Check for the _isDisposed error
        const disposedError = consoleErrors.some(e => e.includes('_isDisposed'));
        if (disposedError) {
            console.log("\nWARNING: _isDisposed error detected (xterm disposal issue)");
        }

        if (!tabBarExists || !tileLayoutExists) {
            console.log("\nBUG DETECTED: UI components destroyed after close!");
            console.log("  - The close operation crashed the React tree");
            process.exit(1);
        }

        // Check if exactly one pane was closed
        const panesRemoved = actualPanesBefore - actualPanesAfter;
        if (actualPanesBefore >= 2) {
            if (panesRemoved === 1 && actualPanesAfter >= 1) {
                console.log("\nSUCCESS: Only one pane was closed as expected");
                process.exit(0);
            } else if (actualPanesAfter === 0) {
                console.log("\nBUG CONFIRMED: All panes closed when only one should have closed!");
                process.exit(1);
            } else {
                console.log(`\nUNEXPECTED: Expected ${actualPanesBefore - 1} panes, got ${actualPanesAfter}`);
                process.exit(1);
            }
        } else {
            console.log("\nTest inconclusive - couldn't create 2 terminals");
            process.exit(2);
        }

    } catch (error) {
        console.error("\nERROR:", error);
        process.exit(1);
    } finally {
        if (electronApp) {
            console.log("\n8. Closing app...");
            await electronApp.close();
            console.log("   Done");
        }
    }
}

main().catch(console.error);
