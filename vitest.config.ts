import { defineConfig, mergeConfig, type UserConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default mergeConfig(
    viteConfig as UserConfig,
    defineConfig({
        test: {
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
