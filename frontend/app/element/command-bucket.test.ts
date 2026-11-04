// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The shell-command rules: the first real program and subcommand decide the bucket. */

import { describe, expect, it } from "vitest";

import type { BucketId } from "./activity-bucket";
import { commandBucketId } from "./command-bucket";

const cases: Array<[string, BucketId | undefined]> = [
    // build and compile
    ["cargo build --release", "build"],
    ["cargo check -p srv", "build"],
    ["tsc --noEmit -p tsconfig.json", "build"],
    ["npx tsc --noEmit -p tsconfig.citypecheck.json", "build"],
    ["vite build", "build"],
    ["task build:frontend", "build"],
    ["npm run build", "build"],
    ["pnpm run bundle:prod", "build"],
    ["make", "build"],
    ["go build ./...", "build"],
    ["docker build -t x .", "build"],
    ["node scripts/gen-app-icons.mjs", "build"],
    // tests and checks
    ["cargo nextest run -p srv", "test"],
    ["cargo test", "test"],
    ["cargo clippy --all-targets", "test"],
    ["vitest run frontend/app", "test"],
    ["node node_modules/vitest/vitest.mjs run scripts/x.test.mjs", "test"],
    ["npm test", "test"],
    ["npm run lint", "test"],
    ["pnpm run test:e2e", "test"],
    ["node scripts/check-function-sizes.mjs", "test"],
    ["bash scripts/check-gates.sh", "test"],
    ["pytest -q", "test"],
    ["eslint .", "test"],
    ["make test", "test"],
    // sending
    ["git push -u origin agent1/x", "send"],
    ["git -C ../other push", "send"],
    ["gh-agent pr create --title x", "send"],
    ["gh-agent -R agentmuxai/agentmux pr merge 12 --auto --squash", "send"],
    ["curl -X POST https://example.com/api", "send"],
    ["curl -T file.bin https://example.com/up", "send"],
    ["scp a.txt host:/tmp", "send"],
    ["npm publish", "send"],
    // receiving
    ["git clone https://github.com/x/y", "receive"],
    ["git fetch origin", "receive"],
    ["git pull", "receive"],
    ["curl -sL https://example.com/file", "receive"],
    ["wget https://example.com/x.zip", "receive"],
    ["gh-agent repo clone a/b", "receive"],
    // install and set up
    ["npm install", "install"],
    ["npm ci", "install"],
    ["pnpm add left-pad", "install"],
    ["cargo fetch", "install"],
    ["pip install -r requirements.txt", "install"],
    ["brew install jq", "install"],
    ["winget install Foo.Bar", "install"],
    ["docker pull ubuntu", "install"],
    ["docker compose up -d", "install"],
    ["bash scripts/setup-dev.sh", "install"],
    // search and scan
    ["rg -n foo src", "search"],
    ["grep -r foo .", "search"],
    ["find . -name '*.rs'", "search"],
    ["git grep -n foo", "search"],
    // deploy and release
    ["cdk deploy Stack", "deploy"],
    ["deploy infra deploy shared", "deploy"],
    ["scripts/release.sh --as patch", "deploy"],
    ["terraform apply", "deploy"],
    ["gh-agent workflow run release.yml", "deploy"],
    // waiting and asleep
    ["gh-agent pr checks 12 --watch", "wait"],
    ["gh-agent run watch 99", "wait"],
    ["sleep 300", "hold"],
    // the first real program, after the scene is set
    ["cd ~/repo && cargo build", "build"],
    ["cd ~/repo && export A=1 && npm test", "test"],
    ["RUST_LOG=debug cargo test", "test"],
    ["timeout 600 cargo build", "build"],
    ["timeout -k 5 600 cargo nextest run", "test"],
    ["time cargo check", "build"],
    ['secrets escalate admin --reason "deploy" -- deploy infra deploy shared', "deploy"],
    ["cargo build 2>&1 | tail -20", "build"],
    ["echo starting; vitest run", "test"],
    // no rule
    ["ls -la", undefined],
    ["git status", undefined],
    ["npm run dev", undefined],
    ["cat file.txt", undefined],
    ["kubectl get pods", undefined],
    ["", undefined],
];

describe("commandBucketId", () => {
    it.each(cases)("%s", (command, id) => {
        expect(commandBucketId(command)).toBe(id);
    });
});
