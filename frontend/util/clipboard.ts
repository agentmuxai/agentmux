// Clipboard utilities — routes through CEF IPC or Tauri plugin depending on host.
import { invokeCommand } from "@/app/platform/ipc";

function isCef(): boolean {
    return typeof (window as any).__AGENTMUX_IPC_PORT__ !== "undefined";
}

export async function readText(): Promise<string> {
    if (isCef()) {
        return invokeCommand<string>("read_clipboard", {});
    }
    const { readText: tauriReadText } = await import("@tauri-apps/plugin-clipboard-manager");
    return (await tauriReadText()) ?? "";
}

export async function writeText(text: string): Promise<void> {
    if (isCef()) {
        await invokeCommand("write_clipboard", { text });
        return;
    }
    const { writeText: tauriWriteText } = await import("@tauri-apps/plugin-clipboard-manager");
    await tauriWriteText(text);
}
