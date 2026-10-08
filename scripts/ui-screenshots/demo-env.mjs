#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Prepares a capture instance's data so screenshots show a made-up project
// instead of the machine's own files — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §8.
//
//   node scripts/ui-screenshots/demo-env.mjs --home <AGENTMUX_HOME_OVERRIDE dir> --demo <dir>
//
// 1. Writes a small demo project ("acme-web") to --demo. The path appears in
//    the Files breadcrumb, the terminal prompt and the editor's title, so it
//    must say nothing about the machine: no user name, nothing under the home
//    folder (e.g. D:/demo/acme-web or /opt/demo/acme-web). It's required, and
//    a path that contains the user name or the home folder is refused.
// 2. Writes a user widgets.json into every channel config folder under --home
//    that starts Hangar (files:path), Terminal (cmd:cwd) and Editor (file) in
//    the demo project, and shows Sysinfo's CPU, memory and network graphs
//    rather than CPU alone. User entries replace built-in ones, so the rest of
//    each entry is copied from the built-in config.
//
// The instance creates its channel config folder on first launch, so run this
// after starting it once; it picks the change up without a restart.

import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir, userInfo } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const WIDGETS_JSON = join(
    dirname(fileURLToPath(import.meta.url)),
    "..",
    "..",
    "crates",
    "srv",
    "src",
    "config",
    "widgets.json"
);

/** The demo project's files, relative path → content. */
export const DEMO_FILES = {
    "README.md":
        "# acme-web\n\nThe storefront for Acme Corp: a small TypeScript web app.\n\n## Develop\n\n    npm install\n    npm run dev\n",
    "CHANGELOG.md": "# Changelog\n\n## 1.4.2\n- Faster checkout\n",
    "package.json":
        JSON.stringify(
            {
                name: "acme-web",
                version: "1.4.2",
                private: true,
                scripts: { dev: "vite", build: "vite build", test: "vitest" },
                dependencies: { "solid-js": "^1.9.0" },
                devDependencies: { typescript: "^5.6.0", vite: "^6.0.0", vitest: "^2.1.0" },
            },
            null,
            2
        ) + "\n",
    "tsconfig.json": '{\n  "compilerOptions": { "strict": true, "target": "ES2022", "module": "ESNext" }\n}\n',
    ".gitignore": "node_modules/\ndist/\n.env\n",
    "src/App.tsx": 'export function App() {\n  return <main class="app">Welcome to Acme</main>;\n}\n',
    "src/main.tsx":
        'import { render } from "solid-js/web";\nimport { App } from "./App";\n\nrender(() => <App />, document.getElementById("root")!);\n',
    "src/components/Cart.tsx":
        'export function Cart(props: { items: number }) {\n  return <span class="cart">{props.items} items</span>;\n}\n',
    "src/components/Header.tsx": "export function Header() {\n  return <header>Acme</header>;\n}\n",
    "src/api/products.ts":
        'export async function getProducts() {\n  const res = await fetch("/api/products");\n  return res.json();\n}\n',
    "docs/architecture.md": "# Architecture\n\nA single-page app talking to the products API.\n",
    "tests/cart.test.ts":
        'import { describe, it, expect } from "vitest";\n\ndescribe("cart", () => {\n  it("counts items", () => expect(1 + 1).toBe(2));\n});\n',
    "public/index.html": '<!doctype html><div id="root"></div>\n',
};

/** The user widgets.json entries that point the file-system widgets at `demo`
 *  (a path with forward slashes). */
export function demoWidgets(demo, builtin = JSON.parse(readFileSync(WIDGETS_JSON, "utf8"))) {
    const copy = (key) => JSON.parse(JSON.stringify(builtin[key]));
    const files = copy("defwidget@files");
    files.blockdef.meta["files:path"] = demo;
    const terminal = copy("defwidget@terminal");
    terminal.blockdef.meta["cmd:cwd"] = demo;
    const editor = copy("defwidget@editor");
    delete editor.blockdef.meta["editor:scratch"];
    // The file tree would be rooted at the home folder, not the demo project.
    editor.blockdef.meta["editor:tree_expanded"] = false;
    editor.blockdef.meta.file = `${demo}/src/App.tsx`;
    // Sysinfo shows CPU only by default; the docs show it full.
    const sysinfo = copy("defwidget@sysinfo");
    sysinfo.blockdef.meta["sysinfo:type"] = "CPU + Mem + Net";
    sysinfo.blockdef.meta["graph:metrics"] = ["cpu", "mem:used", "net:bytestotal"];
    return {
        "defwidget@files": files,
        "defwidget@terminal": terminal,
        "defwidget@editor": editor,
        "defwidget@sysinfo": sysinfo,
    };
}

/** Why `demo` (an absolute path with forward slashes) would identify the
 *  machine in a screenshot, or null if it doesn't: it contains the user name,
 *  or lies under the home folder. Case-insensitive, for Windows paths. */
export function demoPathProblem(demo, { user = userInfo().username, home = homedir() } = {}) {
    const p = demo.toLowerCase();
    const h = home.replace(/\\/g, "/").toLowerCase();
    if (h && (p === h || p.startsWith(h + "/"))) return `it is under the home folder (${home})`;
    if (user && p.split(/[/:]/).some((part) => part.includes(user.toLowerCase())))
        return `it contains the user name (${user})`;
    return null;
}

function main() {
    const args = process.argv.slice(2);
    const get = (flag) => {
        const i = args.indexOf(flag);
        return i >= 0 ? args[i + 1] : undefined;
    };
    const home = get("--home");
    if (!home || !get("--demo")) {
        console.error("usage: demo-env.mjs --home <AGENTMUX_HOME_OVERRIDE dir> --demo <dir with no user name in it>");
        process.exit(2);
    }
    const demo = resolve(get("--demo")).replace(/\\/g, "/");
    const problem = demoPathProblem(demo);
    if (problem) {
        console.error(
            `--demo ${demo} would show in screenshots, and ${problem}; pick a neutral path such as D:/demo/acme-web`
        );
        process.exit(2);
    }
    for (const [rel, content] of Object.entries(DEMO_FILES)) {
        const p = join(demo, rel);
        mkdirSync(dirname(p), { recursive: true });
        writeFileSync(p, content);
    }
    console.log(`demo project: ${demo} (${Object.keys(DEMO_FILES).length} files)`);

    const channels = join(home, "channels");
    const configs = existsSync(channels)
        ? readdirSync(channels)
              .map((c) => join(channels, c, "config"))
              .filter((c) => existsSync(c))
        : [];
    if (configs.length === 0) {
        console.error(`no channel config folder under ${channels}: start the instance once first`);
        process.exit(1);
    }
    const widgets = JSON.stringify(demoWidgets(demo), null, 2) + "\n";
    for (const c of configs) {
        writeFileSync(join(c, "widgets.json"), widgets);
        console.log(`widgets.json: ${join(c, "widgets.json")}`);
    }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
