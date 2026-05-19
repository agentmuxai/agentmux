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
