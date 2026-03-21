// Clipboard utilities using Tauri plugin API.
// Bypasses WebView2's native permission dialog by going through
// Tauri's IPC bridge to the OS clipboard directly.
import { readText as tauriReadText, writeText as tauriWriteText } from "@tauri-apps/plugin-clipboard-manager";

export async function readText(): Promise<string> {
    return (await tauriReadText()) ?? "";
}

export async function writeText(text: string): Promise<void> {
    await tauriWriteText(text);
}
