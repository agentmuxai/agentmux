import { defineConfig, mergeConfig, type UserConfig } from "vitest/config";
import { JSDOM_TS_TESTS } from "./test/jsdom-ts-tests";
import viteConfig from "./vite.config";

const EXCLUDE = [
    "**/node_modules/**",
    "**/dist/**",
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
    "**/.claude/**",
    "**/worktrees/**",
    // `tools/**` holds standalone Node tooling (bench analysis,
    // instance discovery) with `.mjs` tests that aren't frontend and don't run
    // under this jsdom + RPC-mock setup (e.g. tools/tests/lib/
    // bench-stats.test.mjs). They belong to their own runner, not the
    // frontend suite. See SPEC_CI_TEST_RUNNER_2026_06_22.md §6.4.
    "**/tools/**",
];

// Component tests (every `.tsx`, plus the `.ts` files in JSDOM_TS_TESTS)
// run in jsdom; everything else runs in node. Creating a jsdom per test file
// was most of the suite's time when it was the default for all of them.
// A file's own `// @vitest-environment` line still wins over its project.
const JSDOM_INCLUDE = ["**/*.test.tsx", ...JSDOM_TS_TESTS];

export default mergeConfig(
    viteConfig as UserConfig,
    defineConfig({
        test: {
            reporters: ["verbose", "junit"],
            outputFile: {
                junit: "test-results.xml",
            },
            projects: [
                {
                    extends: true,
                    test: {
                        name: "node",
                        environment: "node",
                        exclude: [...EXCLUDE, ...JSDOM_INCLUDE],
                    },
                },
                {
                    extends: true,
                    test: {
                        // SolidJS component tests via @solidjs/testing-library.
                        // Setup adds the @testing-library/jest-dom matchers.
                        // Spec: docs/specs/SPEC_LAUNCH_MODAL_INTEGRATION_TESTS_2026_05_19.md.
                        name: "jsdom",
                        environment: "jsdom",
                        setupFiles: ["./test/vitest-setup.ts"],
                        include: JSDOM_INCLUDE,
                        exclude: EXCLUDE,
                    },
                },
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
