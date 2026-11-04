// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which bucket (activity-bucket.ts) a shell command belongs in, by its first real program and
// subcommand: after `cd ... &&`, environment assignments and wrappers such as `timeout` or
// `secrets escalate ... --`. The word lists are the tables below and nowhere else
// (SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §5).

import type { BucketId } from "./activity-bucket";

/** Programs whose bucket is fixed, whatever their arguments. */
const PROGRAMS: Record<string, BucketId> = {
    // build and compile
    tsc: "build", tsup: "build", webpack: "build", esbuild: "build", rollup: "build", swc: "build", parcel: "build",
    make: "build", cmake: "build", ninja: "build", gn: "build", msbuild: "build", rustc: "build", gcc: "build",
    "g++": "build", clang: "build", "clang++": "build", cc: "build", javac: "build", bazel: "build", gradle: "build",
    gradlew: "build", mvn: "build", mvnw: "build",
    // tests and checks
    vitest: "test", jest: "test", mocha: "test", pytest: "test", "py.test": "test", ava: "test", playwright: "test",
    cypress: "test", nextest: "test", tox: "test", ctest: "test", phpunit: "test", rspec: "test", eslint: "test",
    biome: "test", ruff: "test", mypy: "test", pyright: "test", shellcheck: "test", stylelint: "test",
    // search and scan
    rg: "search", ripgrep: "search", grep: "search", egrep: "search", fgrep: "search", find: "search", fd: "search",
    fdfind: "search", ag: "search", ack: "search", locate: "search", findstr: "search", "select-string": "search",
    // sending
    scp: "send", rsync: "send", sftp: "send", ftp: "send",
    // deploy and release
    deploy: "deploy", terraform: "deploy", tofu: "deploy", pulumi: "deploy", serverless: "deploy", sls: "deploy",
    wrangler: "deploy", vercel: "deploy", netlify: "deploy", flyctl: "deploy", fly: "deploy", fastlane: "deploy",
    helm: "deploy", kubectl: "deploy",
    // asleep
    sleep: "hold", "start-sleep": "hold",
};

/** Programs whose bucket depends on their first non-flag argument (the subcommand). */
const SUBCOMMANDS: Record<string, Record<string, BucketId>> = {
    git: { push: "send", clone: "receive", fetch: "receive", pull: "receive", grep: "search" },
    cargo: {
        build: "build", b: "build", check: "build", c: "build", doc: "build", bench: "build", rustc: "build",
        test: "test", t: "test", nextest: "test", clippy: "test", miri: "test", "llvm-cov": "test",
        fetch: "install", install: "install", add: "install", update: "install",
        publish: "deploy",
    },
    vite: { build: "build" },
    next: { build: "build" },
    go: { build: "build", install: "build", test: "test", vet: "test" },
    dotnet: { build: "build", publish: "build", test: "test", restore: "install" },
    task: { build: "build", test: "test", check: "test", lint: "test" },
    just: { build: "build", test: "test", check: "test", lint: "test" },
    cdk: { deploy: "deploy", destroy: "deploy", synth: "build" },
    docker: { pull: "install", run: "install", start: "install", create: "install", up: "install", load: "install", build: "build", push: "send" },
    podman: { pull: "install", run: "install", start: "install", create: "install", up: "install", load: "install", build: "build", push: "send" },
    compose: { up: "install", pull: "install", build: "build" },
    ...Object.fromEntries(
        ["npm", "pnpm", "yarn", "bun"].map((p) => [
            p,
            { install: "install", i: "install", ci: "install", add: "install", update: "install", up: "install", upgrade: "install", test: "test", t: "test", publish: "send" } as Record<string, BucketId>,
        ])
    ),
    ...Object.fromEntries(
        ["pip", "pip3", "pipx", "uv", "poetry", "conda", "brew", "apt", "apt-get", "dnf", "yum", "zypper", "pacman", "winget", "choco", "scoop", "gem", "bundle", "composer", "nuget", "vcpkg", "conan"].map((p) => [
            p,
            { install: "install", add: "install", update: "install", upgrade: "install", sync: "install", restore: "install", fetch: "install", pull: "install" } as Record<string, BucketId>,
        ])
    ),
};

/** Package runners: the program after them is the one that runs. */
const RUNNERS = new Set(["npx", "bunx", "pnpx"]);
/** `npm run <script>`: the script's own name says what it does. */
const SCRIPT_RUNNERS = new Set(["npm", "pnpm", "yarn", "bun"]);
const SCRIPT_VERBS = new Set(["run", "run-script", "exec", "dlx", "x"]);
/** Programs that run a script file: the file's name says what it does. */
const INTERPRETERS = new Set(["node", "deno", "python", "python3", "py", "bash", "sh", "zsh", "pwsh", "powershell", "ruby", "perl"]);
/** Prefixes that run what follows. */
const WRAPPERS = new Set(["time", "nice", "nohup", "env", "sudo", "command", "exec", "stdbuf", "call"]);
/** Commands that set the scene and are never the work. */
const SCENE_SETTERS = new Set(["cd", "pushd", "popd", "export", "set", "source", ".", "echo", "printf", "true", ":", "unset"]);

const SCRIPT_RULES: ReadonlyArray<[RegExp, BucketId]> = [
    [/(^|[-_./:])(vitest|jest|pytest|tests?|specs?|e2e|lint|typecheck|verify|gates?|checks?)([-_./:]|$)/, "test"],
    [/(^|[-_./:])(deploy|release|publish)([-_./:]|$)/, "deploy"],
    [/(^|[-_./:])(install|setup|bootstrap)([-_./:]|$)/, "install"],
    [/(^|[-_./:])(search|scan|grep)([-_./:]|$)/, "search"],
    [/(^|[-_./:])(build|bundle|compile|package|dist|gen)([-_./:]|$)/, "build"],
];

/** The bucket a script's (or an npm script's) name says, if it says one. */
function byScriptName(name: string): BucketId | undefined {
    const lower = name.toLowerCase();
    return SCRIPT_RULES.find(([re]) => re.test(lower))?.[1];
}

const baseName = (p: string): string => p.replace(/^["']|["']$/g, "").split(/[\\/]/).pop()!.toLowerCase();
const programName = (p: string): string => baseName(p).replace(/\.(exe|cmd|bat|ps1)$/, "");
const isFlag = (t: string): boolean => t.startsWith("-");

/** The tokens of the first command that does work: scene-setting segments, assignments and wrappers dropped. */
function workTokens(command: string): string[] {
    for (const segment of command.split(/&&|\|\||;|\||\r?\n/)) {
        let tokens = segment.trim().split(/\s+/).filter(Boolean);
        while (tokens.length) {
            const head = programName(tokens[0]);
            if (/^[A-Za-z_]\w*=/.test(tokens[0])) tokens = tokens.slice(1);
            else if (head === "timeout") tokens = tokens.slice(1).filter((t, i, all) => !isFlag(t) && all.slice(0, i).every(isFlag)).length ? dropFirstOperand(tokens.slice(1)) : tokens.slice(1);
            else if (head === "secrets" && tokens.includes("--")) tokens = tokens.slice(tokens.indexOf("--") + 1);
            else if (WRAPPERS.has(head)) tokens = tokens.slice(1);
            else break;
        }
        if (tokens.length && !SCENE_SETTERS.has(programName(tokens[0]))) return tokens;
    }
    return [];
}

/** `timeout 30 cmd`: drop the flags and the duration that follow `timeout`. */
function dropFirstOperand(rest: string[]): string[] {
    const flagsEnd = rest.findIndex((t) => !isFlag(t));
    return flagsEnd < 0 ? [] : rest.slice(flagsEnd + 1);
}

/** The tool word a script's `node_modules/<tool>/...` path names, for `node node_modules/vitest/vitest.mjs`. */
function toolInPath(path: string): string | undefined {
    return path.replace(/\\/g, "/").match(/node_modules\/(?:@[^/]+\/)?([^/]+)\//)?.[1];
}

function interpreted(program: string, args: string[]): BucketId | undefined {
    const moduleIdx = args.indexOf("-m");
    if (moduleIdx >= 0 && args[moduleIdx + 1]) return classify([args[moduleIdx + 1], ...args.slice(moduleIdx + 2)]);
    const script = args.find((t) => !isFlag(t));
    if (!script) return undefined;
    const tool = toolInPath(script);
    return (tool && classify([tool, ...args.slice(args.indexOf(script) + 1)])) || byScriptName(baseName(script)) || (program === "node" ? undefined : undefined);
}

/** `gh` and `gh-agent`: sending is a write, watching is waiting. */
function viaGitHub(args: string[]): BucketId | undefined {
    const words: string[] = [];
    for (let i = 0; i < args.length; i++) {
        if (args[i] === "-R" || args[i] === "--repo") i++;
        else if (!isFlag(args[i])) words.push(args[i]);
    }
    const [a, b] = words;
    const watching = args.includes("--watch") || b === "watch";
    if ((a === "pr" && b === "checks" && watching) || (a === "run" && b === "watch")) return "wait";
    if (a === "pr" && ["create", "edit", "comment", "review", "merge", "ready", "close", "reopen"].includes(b)) return "send";
    if (a === "issue" && ["create", "comment"].includes(b)) return "send";
    if (a === "repo" && b === "clone") return "receive";
    if ((a === "release" && b === "create") || (a === "workflow" && b === "run")) return "deploy";
    return undefined;
}

const UPLOAD_FLAGS = /^(-T|--upload-file|-F|--form|-d|--data(-\w+)?|--json)$/;

/** `curl` and `wget`: receiving, unless the flags say it uploads. */
function viaHttp(args: string[]): BucketId {
    const method = args[args.indexOf("-X") + 1]?.toUpperCase();
    const writes = args.some((t) => UPLOAD_FLAGS.test(t)) || (args.includes("-X") && method !== "GET" && method !== "HEAD");
    return writes ? "send" : "receive";
}

function classify(tokens: string[]): BucketId | undefined {
    if (tokens.length === 0) return undefined;
    const program = programName(tokens[0]);
    const args = tokens.slice(1);
    if (RUNNERS.has(program)) return classify(args.filter((t, i) => !isFlag(t) || args.slice(0, i).every(isFlag)).filter((t) => !isFlag(t)));
    if (INTERPRETERS.has(program)) return interpreted(program, args);
    if (program === "gh" || program === "gh-agent") return viaGitHub(args);
    if (["curl", "wget", "iwr", "invoke-webrequest", "http", "https"].includes(program)) return viaHttp(args);
    const sub = args.find((t) => !isFlag(t));
    if (SCRIPT_RUNNERS.has(program) && sub) {
        if (SCRIPT_VERBS.has(sub)) {
            const rest = args.slice(args.indexOf(sub) + 1);
            const name = rest.find((t) => !isFlag(t));
            if (sub === "run" || sub === "run-script") return name ? byScriptName(name) : undefined;
            return name ? classify(rest.filter((t) => !isFlag(t) || rest.indexOf(t) > rest.indexOf(name))) : undefined;
        }
        return SUBCOMMANDS[program][sub] ?? byScriptName(sub);
    }
    if (program === "docker" && sub === "compose") return classify(["compose", ...args.slice(args.indexOf(sub) + 1)]);
    if (sub && SUBCOMMANDS[program]) return SUBCOMMANDS[program][sub];
    if (program === "make" || program === "task" || program === "just") return (sub && byScriptName(sub)) || PROGRAMS[program];
    return PROGRAMS[program] ?? byScriptName(baseName(tokens[0]));
}

/** The bucket id a shell command belongs in, or undefined when no rule matches it. */
export function commandBucketId(command: string): BucketId | undefined {
    return classify(workTokens(command));
}
