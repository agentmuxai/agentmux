// Clipboard utilities — routes through CEF IPC to the OS clipboard.
// CEF's Chromium blocks navigator.clipboard.readText() without a
// Permissions-Policy header, so we use the host process instead.
import { getApi } from "@/app/store/app-api";

export async function readText(): Promise<string> {
    return getApi().readClipboardText();
}

export async function writeText(text: string): Promise<void> {
    await getApi().writeClipboardText(text);
}
