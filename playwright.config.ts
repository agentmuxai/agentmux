import { defineConfig } from "playwright/test";

export default defineConfig({
    testDir: "./e2e",
    timeout: 120000,
    globalTimeout: 300000,
    expect: {
        timeout: 10000,
    },
    fullyParallel: false,
    forbidOnly: !!process.env.CI,
    retries: process.env.CI ? 2 : 0,
    workers: 1,
    reporter: "html",
    use: {
        trace: "on-first-retry",
        video: "on-first-retry",
    },
});
