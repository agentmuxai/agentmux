// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What a tile shows for each kind of attachment: a thumbnail when the preview
 * is cheap and safe (images, SVG, text), a colored type icon otherwise.
 * srv decides the kind from the file's content (`AttachmentInfo.kind`); the
 * transcript only has names, so it guesses from the extension.
 * docs/specs/SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §5.
 */

export type FileKind =
    | "image"
    | "svg"
    | "text"
    | "pdf"
    | "word"
    | "excel"
    | "powerpoint"
    | "archive"
    | "audio"
    | "video"
    | "image_file"
    | "other";

const KINDS = new Set<string>([
    "image",
    "svg",
    "text",
    "pdf",
    "word",
    "excel",
    "powerpoint",
    "archive",
    "audio",
    "video",
    "image_file",
    "other",
]);

const BY_EXT: Record<string, FileKind> = {};
const add = (kind: FileKind, exts: string) => exts.split(" ").forEach((e) => (BY_EXT[e] = kind));
add("image", "png jpg jpeg jfif gif webp bmp dib tif tiff ico");
add("image_file", "heic heif avif");
add("svg", "svg");
add("pdf", "pdf");
add("word", "doc docx docm dot dotx odt rtf");
add("excel", "xls xlsx xlsm xlt ods");
add("powerpoint", "ppt pptx pptm pps ppsx pot odp");
add("archive", "zip gz tgz tar 7z rar bz2 xz zst");
add("audio", "mp3 wav flac m4a aac ogg oga opus wma aiff aif");
add("video", "mp4 m4v mov webm mkv avi wmv flv mpg mpeg");
add(
    "text",
    "txt text md markdown mdx rst adoc csv tsv json jsonl ndjson yaml yml toml ini cfg conf env properties xml html htm " +
        "css scss sass less log rs ts tsx js jsx mjs cjs py pyi go java kt kts scala c h cc cpp cxx hpp hh cs fs rb php " +
        "swift m mm dart lua pl r jl ex exs erl hs clj sh bash zsh fish ps1 psm1 bat cmd sql graphql gql proto vue svelte " +
        "astro tf hcl gradle cmake make mk dockerfile gitignore diff patch tex bib srt vtt ipynb",
);

/** The file name's extension, lowercased, without the dot. */
export function extensionOf(name: string): string {
    const m = /\.([A-Za-z0-9]{1,10})$/.exec(name);
    return m ? m[1].toLowerCase() : "";
}

/** srv's kind when known, else a guess from the name. */
export function fileKind(kind: string | undefined, name: string): FileKind {
    if (kind && KINDS.has(kind)) return kind as FileKind;
    return BY_EXT[extensionOf(name)] ?? "other";
}

/** Kinds whose tile is a thumbnail rather than an icon. */
export function hasThumbnail(kind: FileKind): boolean {
    return kind === "image" || kind === "svg" || kind === "text";
}

/** Kinds the lightbox shows as a picture. */
export function isPicture(kind: FileKind): boolean {
    return kind === "image" || kind === "svg";
}

const CODE_EXTS = new Set(
    "rs ts tsx js jsx mjs cjs py pyi go java kt kts scala c h cc cpp cxx hpp hh cs fs rb php swift m mm dart lua pl r jl ex exs erl hs clj sh bash zsh fish ps1 psm1 bat cmd sql graphql gql proto vue svelte astro tf hcl html htm css scss sass less json xml yaml yml toml".split(
        " ",
    ),
);

/** Font Awesome icon for a kind; text falls back to csv/code/lines by extension. */
export function kindIcon(kind: FileKind, name: string): string {
    switch (kind) {
        case "pdf":
            return "fa-file-pdf";
        case "word":
            return "fa-file-word";
        case "excel":
            return "fa-file-excel";
        case "powerpoint":
            return "fa-file-powerpoint";
        case "archive":
            return "fa-file-zipper";
        case "audio":
            return "fa-file-audio";
        case "video":
            return "fa-file-video";
        case "image":
        case "image_file":
        case "svg":
            return "fa-file-image";
        case "text": {
            const ext = extensionOf(name);
            if (ext === "csv" || ext === "tsv") return "fa-file-csv";
            return CODE_EXTS.has(ext) ? "fa-file-code" : "fa-file-lines";
        }
        default:
            return "fa-file";
    }
}

/** A short, human label for the lightbox and tooltips. */
export function kindLabel(kind: FileKind): string {
    switch (kind) {
        case "image":
            return "Image";
        case "svg":
            return "SVG image";
        case "text":
            return "Text";
        case "pdf":
            return "PDF";
        case "word":
            return "Document";
        case "excel":
            return "Spreadsheet";
        case "powerpoint":
            return "Presentation";
        case "archive":
            return "Archive";
        case "audio":
            return "Audio";
        case "video":
            return "Video";
        case "image_file":
            return "Image (not previewed)";
        default:
            return "File";
    }
}

/** The label under an icon: `DOCX`, `ZIP`, …; empty when there is no extension. */
export function extLabel(name: string): string {
    return extensionOf(name).slice(0, 5).toUpperCase();
}

/** "image"/"images" when every attachment is an image, else "attachment(s)". */
export function attachmentNoun(n: number, kinds: FileKind[]): string {
    const images = kinds.length > 0 && kinds.every((k) => k === "image");
    if (images) return n === 1 ? "image" : "images";
    return n === 1 ? "attachment" : "attachments";
}
