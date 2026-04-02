// Clipboard utilities — routes through CEF IPC to the OS clipboard.
// CEF's Chromium blocks navigator.clipboard.readText() without a
// Permissions-Policy header, so we use the host process instead.
import { invokeCommand } from "@/app/platform/ipc";

export async function readText(): Promise<string> {
    return invokeCommand<string>("read_clipboard", {});
}

export async function writeText(text: string): Promise<void> {
    await invokeCommand("write_clipboard", { text });
}
