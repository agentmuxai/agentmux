import { defineConfig, mergeConfig, type UserConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default mergeConfig(
    viteConfig as UserConfig,
    defineConfig({
        test: {
            // SolidJS component tests run in jsdom via
            // @solidjs/testing-library. Pure-reducer unit tests
            // don't need jsdom but tolerate it. Setup adds
            // @testing-library/jest-dom matchers and global RPC
            // mocks. Spec:
            // docs/specs/SPEC_LAUNCH_MODAL_INTEGRATION_TESTS_2026_05_19.md.
            environment: "jsdom",
            setupFiles: ["./test/vitest-setup.ts"],
            reporters: ["verbose", "junit"],
            outputFile: {
                junit: "test-results.xml",
            },
            exclude: [
                "**/node_modules/**",
                "**/dist/**",
                "**/infra/cdk/**", // CDK has its own testing setup with aws-cdk-lib
                // Leftover git worktrees from prior agent sessions
                // live under `.claude/worktrees/agent-*/`. Each carries
                // a full clone of the project (including test files),
                // so without an exclusion vitest discovers and runs
                // them all — inflating runtime ~6× and showing each
                // real failure under N duplicate paths.
                //
                // `**/.claude/**` is the precise rule for the current
                // layout; `**/worktrees/**` is a defensive second net
                // in case the location moves.
                // Spec: docs/specs/SPEC_FRONTEND_TEST_HEALTH_2026_05_24.md §1.
                "**/.claude/**",
                "**/worktrees/**",
            ],
            coverage: {
                provider: "istanbul",
                reporter: ["lcov"],
                reportsDirectory: "./coverage",
            },
            typecheck: {
                tsconfig: "tsconfig.json",
            },
        },
    })
);
