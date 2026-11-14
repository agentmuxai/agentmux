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
