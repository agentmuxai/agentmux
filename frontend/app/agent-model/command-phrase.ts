// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A shell command in words, for the rows that name what an agent is running when the call came with no
 * `description`: the Activity Dock, the working row. Never the command line itself, which is mostly a `cd` into a
 * workspace path, an environment prefix or a wrapper. It skips those, names the real program and its verb
 * ("Run cargo test in rr", "Build with npm"), and shortens a folder to its last name. The home path and an agent's
 * workspace path are never shown. The caller keeps the full command for the tooltip and the expanded row.
 */

const MAX_PHRASE = 80;
const MAX_DEPTH = 2;

/** Programs that run a script or code: the name of the script is what the call is about. */
const INTERPRETERS = new Set([
    "node",
    "python",
    "python3",
    "py",
    "bash",
    "sh",
    "zsh",
    "pwsh",
    "powershell",
    "ruby",
    "deno",
    "bun",
    "tsx",
    "ts-node",
]);

/** Programs whose first plain argument is a subcommand worth naming. */
const SUBCOMMANDS = new Set([
    "cargo",
    "git",
    "gh",
    "gh-agent",
    "docker",
    "kubectl",
    "go",
    "dotnet",
    "task",
    "systemctl",
    "brew",
    "apt",
    "apt-get",
    "winget",
    "secrets",
    "aws",
    "gcloud",
    "az",
    "terraform",
    "rustup",
    "npx",
    "make",
    "just",
    "cmake",
]);

const PACKAGE_MANAGERS = new Set(["npm", "pnpm", "yarn", "bun"]);

/** Everyday programs, in a few plain words. */
const PLAIN: Record<string, string> = {
    ls: "List files",
    dir: "List files",
    cat: "Read a file",
    head: "Read a file",
    tail: "Read a file",
    type: "Read a file",
    grep: "Search files",
    rg: "Search files",
    find: "Find files",
    sleep: "Wait",
    curl: "Fetch a URL",
    wget: "Fetch a URL",
    mkdir: "Make a folder",
    rm: "Remove files",
    cp: "Copy files",
    mv: "Move files",
    echo: "Print text",
    sed: "Edit text",
    awk: "Process text",
    diff: "Compare files",
    tar: "Pack or unpack files",
    unzip: "Unpack an archive",
    zip: "Pack files",
};

const PACKAGE_VERBS: Record<string, string> = {
    test: "Run tests with",
    build: "Build with",
    install: "Install packages with",
    ci: "Install packages with",
    i: "Install packages with",
    add: "Install packages with",
    lint: "Lint with",
    start: "Start with",
    dev: "Start with",
};

/** Segments the command is made of, split on `&&`, `||`, `;`, `|` and line breaks outside quotes. */
function segments(command: string): string[] {
    const out: string[] = [];
    let cur = "";
    let quote = "";
    for (let i = 0; i < command.length; i++) {
        const c = command[i];
        if (quote) {
            cur += c;
            if (c === quote) quote = "";
            continue;
        }
        if (c === '"' || c === "'") {
            quote = c;
            cur += c;
        } else if (c === "\n" || c === ";" || c === "|" || (c === "&" && command[i + 1] === "&")) {
            if (c === "&" || (c === "|" && command[i + 1] === "|")) i++;
            if (cur.trim()) out.push(cur.trim());
            cur = "";
        } else if (c === "\r") {
            continue;
        } else {
            cur += c;
        }
    }
    if (cur.trim()) out.push(cur.trim());
    return out;
}

/** Words of one segment, quotes removed. */
function words(segment: string): string[] {
    const out: string[] = [];
    const re = /"([^"]*)"|'([^']*)'|(\S+)/g;
    for (let m = re.exec(segment); m; m = re.exec(segment)) out.push(m[1] ?? m[2] ?? m[3]);
    return out;
}

const isEnvAssignment = (w: string) => /^[A-Za-z_][A-Za-z0-9_]*=/.test(w);

function baseName(p: string): string {
    return p.split(/[\\/]/).filter(Boolean).at(-1) ?? p;
}

/** The program a word names: its file name, lower-cased, without an executable extension. */
function programOf(w: string): string {
    return baseName(w)
        .replace(/\.(exe|cmd|bat|ps1|sh|py|mjs|cjs|js|ts)$/i, "")
        .toLowerCase();
}

/** A folder's last name, unless it is the home folder or an agent's workspace (nothing worth naming). */
function folderName(dir: string): string | null {
    const parts = dir.split(/[\\/]/).filter((p) => p && p !== "." && p !== "..");
    const last = parts.at(-1);
    if (!last || last === "~" || last === "$HOME" || /^[A-Za-z]:$/.test(last)) return null;
    if (parts.at(-2) === "agents") return null;
    if (parts.at(-2) === "Users" || parts.at(-2) === "home") return null;
    return last;
}

/** A wrapper's own flags (`sudo -E`) are not the program. */
function skipFlags(ws: string[]): string[] {
    let w = ws;
    while (w[0]?.startsWith("-")) w = w.slice(1);
    return w;
}

/** What the leading words of a segment hide: environment assignments and wrappers. Returns the rest. */
function unwrap(ws: string[]): string[] {
    let w = ws;
    for (;;) {
        const first = w[0];
        if (first === undefined) return w;
        const name = programOf(first);
        if (isEnvAssignment(first)) w = w.slice(1);
        else if (["sudo", "time", "nohup", "env", "command", "exec", "nice", "call"].includes(name))
            w = skipFlags(w.slice(1));
        else if (name === "timeout") {
            w = skipFlags(w.slice(1));
            if (w[0] && /^\d/.test(w[0])) w = w.slice(1);
        } else if ((name === "secrets" && w[1] === "escalate") || (name === "gh-agent" && w[1] === "sudo")) {
            const dash = w.indexOf("--");
            if (dash === -1) return w;
            w = w.slice(dash + 1);
        } else return w;
    }
}

function phraseSegment(ws: string[], dir: string | null, depth: number): string {
    const w = unwrap(ws);
    const first = w[0];
    if (first === undefined) return "";
    const program = programOf(first);
    const where = dir ? ` in ${dir}` : "";
    const args = w.slice(1);
    const plain = args.filter((a) => !a.startsWith("-"));

    // A shell given a command string: the string is what runs.
    const dashC = args.findIndex((a) => /^-(c|lc|command)$/i.test(a));
    if (
        ["bash", "sh", "zsh", "pwsh", "powershell"].includes(program) &&
        dashC !== -1 &&
        args[dashC + 1] &&
        depth < MAX_DEPTH
    ) {
        const inner = phraseCommandAt(args[dashC + 1], depth + 1, dir);
        if (inner) return inner;
    }
    // eval "$(...)" runs what the substitution holds.
    if (program === "eval" && args.length > 0 && depth < MAX_DEPTH) {
        const inner = phraseCommandAt(args.join(" ").replace(/^\$\(/, "").replace(/\)$/, ""), depth + 1, dir);
        if (inner) return inner;
    }
    if (INTERPRETERS.has(program)) {
        const script = plain[0];
        return script
            ? `Run ${baseName(script).replace(/\.(mjs|cjs|js|ts|py|sh|ps1)$/i, "")}${where}`
            : `Run ${program}${where}`;
    }
    if (PACKAGE_MANAGERS.has(program)) {
        const verb = plain[0] === "run" || plain[0] === "run-script" ? plain[1] : plain[0];
        const said = verb ? PACKAGE_VERBS[verb] : undefined;
        if (said) return `${said} ${program}${where}`;
        return verb
            ? `Run ${program} ${plain[0] === "run" || plain[0] === "run-script" ? `script ${verb}` : verb}${where}`
            : `Run ${program}${where}`;
    }
    if (SUBCOMMANDS.has(program)) {
        const verb = plain.find((a) => /^[a-z][a-z0-9-]{0,18}$/.test(a));
        return `Run ${program}${verb ? ` ${verb}` : ""}${where}`;
    }
    const said = PLAIN[program];
    if (said) return `${said}${where}`;
    return `Run ${/^[\w.+-]+$/.test(program) ? program : "a program"}${where}`;
}

function phraseCommandAt(command: string, depth: number, outerDir: string | null): string {
    let dir = outerDir;
    for (const seg of segments(command)) {
        const ws = words(seg);
        const first = ws[0];
        if (first === undefined) continue;
        const name = programOf(first);
        if (name === "cd" || name === "pushd" || name === "set-location" || name === "sl") {
            const target = ws.slice(1).find((a) => !/^\/[a-z]$/i.test(a) && !a.startsWith("-"));
            if (target) dir = folderName(target);
            continue;
        }
        if (name === "export" || name === "set" || name === "source" || name === "." || ws.every(isEnvAssignment))
            continue;
        const phrase = phraseSegment(ws, dir, depth);
        if (phrase) return phrase;
    }
    return "";
}

/** The command in words, at most 80 characters; "Run a shell command" when nothing in it can be named. */
export function phraseCommand(command: string | undefined | null): string {
    const phrase = phraseCommandAt(command ?? "", 0, null) || "Run a shell command";
    return phrase.length > MAX_PHRASE ? `${phrase.slice(0, MAX_PHRASE - 1)}…` : phrase;
}
