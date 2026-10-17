// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The file tree's icon for an entry: a Font Awesome icon that says what kind
// of file it is (the language's own mark where Font Awesome has one) and a
// colour from VS Code's default Seti file icons, so a folder reads at a glance
// the way it does there. Colours are `--file-icon-<tone>` tokens (theme.scss).

export type FileIconTone = "blue" | "green" | "yellow" | "orange" | "red" | "purple" | "pink" | "steel" | "grey";

export interface FileIcon {
    /** Font Awesome classes, e.g. `fa-brands fa-python`. */
    icon: string;
    tone: FileIconTone;
}

const solid = (name: string, tone: FileIconTone): FileIcon => ({ icon: `fa-solid fa-${name}`, tone });
const brand = (name: string, tone: FileIconTone): FileIcon => ({ icon: `fa-brands fa-${name}`, tone });

/** Whole file names that say more than their extension. Lower case. */
const BY_NAME: Record<string, FileIcon> = {
    "package.json": brand("npm", "red"),
    "package-lock.json": brand("npm", "grey"),
    dockerfile: brand("docker", "blue"),
    "docker-compose.yml": brand("docker", "blue"),
    "docker-compose.yaml": brand("docker", "blue"),
    ".gitignore": brand("git-alt", "orange"),
    ".gitattributes": brand("git-alt", "orange"),
    ".gitmodules": brand("git-alt", "orange"),
    "cargo.toml": brand("rust", "orange"),
    "cargo.lock": solid("lock", "grey"),
    "go.mod": brand("golang", "blue"),
    "go.sum": solid("lock", "grey"),
    makefile: solid("gears", "orange"),
    "taskfile.yml": solid("gears", "purple"),
    license: solid("scale-balanced", "yellow"),
    "license.md": solid("scale-balanced", "yellow"),
    "license.txt": solid("scale-balanced", "yellow"),
    readme: solid("circle-info", "blue"),
    "readme.md": solid("circle-info", "blue"),
    ".env": solid("key", "yellow"),
};

/** By extension (lower case, no dot). */
const BY_EXT: Record<string, FileIcon> = {
    ts: solid("file-code", "blue"),
    mts: solid("file-code", "blue"),
    cts: solid("file-code", "blue"),
    tsx: brand("react", "blue"),
    jsx: brand("react", "yellow"),
    js: brand("js", "yellow"),
    mjs: brand("js", "yellow"),
    cjs: brand("js", "yellow"),
    py: brand("python", "blue"),
    pyi: brand("python", "blue"),
    ipynb: solid("book", "orange"),
    rs: brand("rust", "orange"),
    go: brand("golang", "blue"),
    java: brand("java", "red"),
    kt: solid("file-code", "purple"),
    c: solid("c", "blue"),
    h: solid("c", "purple"),
    cpp: solid("c", "blue"),
    cc: solid("c", "blue"),
    hpp: solid("c", "purple"),
    cs: solid("hashtag", "blue"),
    swift: brand("swift", "orange"),
    php: brand("php", "purple"),
    rb: solid("gem", "red"),
    lua: solid("moon", "blue"),
    sh: solid("terminal", "green"),
    bash: solid("terminal", "green"),
    zsh: solid("terminal", "green"),
    fish: solid("terminal", "green"),
    ps1: solid("terminal", "blue"),
    bat: solid("terminal", "yellow"),
    cmd: solid("terminal", "yellow"),
    html: brand("html5", "orange"),
    htm: brand("html5", "orange"),
    css: brand("css3-alt", "blue"),
    scss: brand("sass", "pink"),
    sass: brand("sass", "pink"),
    less: brand("css3-alt", "purple"),
    vue: solid("file-code", "green"),
    svelte: solid("file-code", "orange"),
    json: solid("file-code", "yellow"),
    jsonc: solid("file-code", "yellow"),
    json5: solid("file-code", "yellow"),
    yaml: solid("gear", "purple"),
    yml: solid("gear", "purple"),
    toml: solid("gear", "steel"),
    ini: solid("gear", "steel"),
    conf: solid("gear", "steel"),
    cfg: solid("gear", "steel"),
    env: solid("key", "yellow"),
    xml: solid("code", "orange"),
    md: brand("markdown", "blue"),
    markdown: brand("markdown", "blue"),
    mdx: brand("markdown", "yellow"),
    rst: solid("file-lines", "steel"),
    txt: solid("file-lines", "steel"),
    log: solid("file-lines", "grey"),
    csv: solid("file-csv", "green"),
    tsv: solid("file-csv", "green"),
    xlsx: solid("file-excel", "green"),
    xls: solid("file-excel", "green"),
    docx: solid("file-word", "blue"),
    doc: solid("file-word", "blue"),
    pptx: solid("file-powerpoint", "orange"),
    ppt: solid("file-powerpoint", "orange"),
    pdf: solid("file-pdf", "red"),
    sql: solid("database", "pink"),
    db: solid("database", "pink"),
    sqlite: solid("database", "pink"),
    svg: solid("file-image", "yellow"),
    png: solid("file-image", "purple"),
    jpg: solid("file-image", "purple"),
    jpeg: solid("file-image", "purple"),
    gif: solid("file-image", "purple"),
    webp: solid("file-image", "purple"),
    bmp: solid("file-image", "purple"),
    ico: solid("file-image", "purple"),
    heic: solid("file-image", "purple"),
    mp3: solid("file-audio", "pink"),
    wav: solid("file-audio", "pink"),
    flac: solid("file-audio", "pink"),
    ogg: solid("file-audio", "pink"),
    m4a: solid("file-audio", "pink"),
    aac: solid("file-audio", "pink"),
    mp4: solid("file-video", "pink"),
    webm: solid("file-video", "pink"),
    mov: solid("file-video", "pink"),
    mkv: solid("file-video", "pink"),
    zip: solid("file-zipper", "yellow"),
    tar: solid("file-zipper", "yellow"),
    gz: solid("file-zipper", "yellow"),
    tgz: solid("file-zipper", "yellow"),
    "7z": solid("file-zipper", "yellow"),
    rar: solid("file-zipper", "yellow"),
    bz2: solid("file-zipper", "yellow"),
    xz: solid("file-zipper", "yellow"),
    lock: solid("lock", "grey"),
    pem: solid("key", "yellow"),
    key: solid("key", "yellow"),
    crt: solid("certificate", "yellow"),
    ttf: solid("font", "red"),
    otf: solid("font", "red"),
    woff: solid("font", "red"),
    woff2: solid("font", "red"),
    exe: solid("cube", "grey"),
    dll: solid("cube", "grey"),
    so: solid("cube", "grey"),
    dylib: solid("cube", "grey"),
    wasm: solid("cube", "purple"),
    bin: solid("cube", "grey"),
};

const FILE = solid("file", "grey");

export function fileIcon(name: string, isDir: boolean, expanded = false): FileIcon {
    if (isDir) return solid(expanded ? "folder-open" : "folder", "steel");
    const lower = name.toLowerCase();
    const byName = BY_NAME[lower];
    if (byName) return byName;
    if (lower.startsWith(".env.")) return BY_NAME[".env"];
    const dot = lower.lastIndexOf(".");
    if (dot <= 0) return FILE;
    return BY_EXT[lower.slice(dot + 1)] ?? FILE;
}
