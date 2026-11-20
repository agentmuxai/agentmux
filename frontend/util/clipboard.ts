// Clipboard utilities — routes through CEF IPC to the OS clipboard.
// CEF's Chromium blocks navigator.clipboard.readText() without a
// Permissions-Policy header, so we use the host process instead.
import { getApi } from "@/util/app-api";

export async function readText(): Promise<string> {
    return getApi().readClipboardText();
}

/** Text plus file/image paths on the clipboard (see readClipboardAttachments). */
export async function readAttachments(): Promise<{ text: string; paths: string[] }> {
    return getApi().readClipboardAttachments();
}

export async function writeText(text: string): Promise<void> {
    try {
        await getApi().writeClipboardText(text);
    } catch (err) {
        // The host reaches the OS clipboard through a tool (wl-copy, xclip or xsel on Linux). With none
        // installed, the page's own clipboard still takes a copy that a click started.
        if (!writeThroughPage(text)) throw err;
    }
}

/** Base64 of `blob`'s bytes, for the host (it takes JSON). */
function toBase64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
        reader.onerror = () => reject(reader.error ?? new Error("could not read the image"));
        reader.readAsDataURL(blob);
    });
}

/**
 * Put a PNG on the clipboard as an image. The host does it where it can (the embedded browser's own call wants the
 * window focused and refuses what it doesn't like); a page with no host, or one whose host can't (no `wl-copy` or
 * `xclip` on Linux), uses the browser's.
 */
export async function writeImage(png: Blob): Promise<void> {
    const host = getApi()?.writeClipboardImage;
    let hostError: unknown;
    if (typeof host === "function") {
        try {
            await host(await toBase64(png));
            return;
        } catch (e) {
            hostError = e;
        }
    }
    if (typeof ClipboardItem !== "undefined" && navigator.clipboard?.write) {
        try {
            await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
            return;
        } catch (e) {
            hostError ??= e;
        }
    }
    throw hostError ?? new Error("this window can't copy an image");
}

/** Copy `text` with the page's own clipboard (a selection copy, which a click may start). */
function writeThroughPage(text: string): boolean {
    if (typeof document === "undefined" || typeof document.execCommand !== "function") return false;
    const box = document.createElement("textarea");
    box.value = text;
    box.setAttribute("readonly", "");
    box.style.position = "fixed";
    box.style.opacity = "0";
    const holder = document.activeElement as HTMLElement | null;
    document.body.appendChild(box);
    try {
        box.select();
        return document.execCommand("copy");
    } catch {
        return false;
    } finally {
        box.remove();
        holder?.focus?.();
    }
}
