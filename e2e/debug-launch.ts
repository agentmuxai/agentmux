import { _electron as electron } from "@playwright/test";
import path from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

async function main() {
    const appPath = path.join(__dirname, "..", "make", "win-unpacked", "AgentMux.exe");
    console.log("Launching:", appPath);

    const electronApp = await electron.launch({
        executablePath: appPath,
        args: [],
        timeout: 120000,
    });

    console.log("App launched!");

    // List all windows
    const windows = electronApp.windows();
    console.log("Number of windows:", windows.length);

    for (let i = 0; i < windows.length; i++) {
        const win = windows[i];
        console.log(`Window ${i}: ${await win.title()}`);
    }

    // Try to get first window
    console.log("Waiting for first window...");
    const window = await electronApp.firstWindow();
    console.log("Got window:", await window.title());

    // Take screenshot
    await window.screenshot({ path: "e2e/screenshots/debug.png" });
    console.log("Screenshot saved");

    await electronApp.close();
    console.log("Done");
}

main().catch(console.error);
