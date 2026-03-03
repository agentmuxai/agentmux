// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Native notification integration for Tauri
// Wraps tauri-plugin-notification for OS-level notifications

import { isNativePresent, sendNotification as tauriSendNotification } from "@tauri-apps/plugin-notification";

export interface NativeNotificationOptions {
    /**
     * The title of the notification
     */
    title: string;

    /**
     * The body content of the notification
     */
    body?: string;

    /**
     * The icon path (optional)
     */
    icon?: string;

    /**
     * The notification sound (optional)
     */
    sound?: string;

    /**
     * The large icon path (Android only)
     */
    largeIcon?: string;

    /**
     * The small icon path (Android only)
     */
    smallIcon?: string;
}

/**
 * Check if native notifications are available
 * @returns true if running in Tauri with notification plugin enabled
 */
export function isNativeNotificationAvailable(): Promise<boolean> {
    try {
        return isNativePresent();
    } catch {
        return Promise.resolve(false);
    }
}

/**
 * Send a native OS notification via Tauri
 * Falls back to in-app notification if native is not available
 *
 * @param options Notification options
 * @returns Promise that resolves when notification is sent
 */
export async function sendNativeNotification(options: NativeNotificationOptions): Promise<void> {
    // Check if native notifications are available
    if (await isNativeNotificationAvailable()) {
        try {
            await tauriSendNotification(options);
            console.log("Native notification sent:", options.title);
        } catch (error) {
            console.warn("Failed to send native notification:", error);
        }
    } else {
        console.log("Native notifications not available");
    }

    // Always log to console for debugging
    console.log(`Notification: ${options.title}`, options.body);
}

/**
 * Request notification permission (if needed on the platform)
 * @returns Promise<"granted" | "denied" | "default">
 */
export async function requestNotificationPermission(): Promise<"granted" | "denied" | "default"> {
    if (!(await isNativeNotificationAvailable())) {
        return "default";
    }

    // Tauri handles permissions automatically in most cases
    // This is here for future extensibility if manual permission is needed
    return "granted";
}

/**
 * Send a notification when a command completes (terminal use case)
 *
 * @param command The command that completed
 * @param success Whether the command succeeded
 * @param duration How long the command took (optional)
 */
export async function notifyCommandComplete(command: string, success: boolean, duration?: number): Promise<void> {
    const title = success ? "Command Completed" : "Command Failed";
    const truncatedCmd = command.length > 50 ? command.substring(0, 47) + "..." : command;

    let body = `$ ${truncatedCmd}`;
    if (duration) {
        const seconds = (duration / 1000).toFixed(1);
        body += `\n⏱️ ${seconds}s`;
    }

    await sendNativeNotification({
        title,
        body,
        icon: success ? undefined : "error",
    });
}

/**
 * Send a notification when a long-running task completes in background
 *
 * @param taskName Name of the task
 * @param message Completion message
 */
export async function notifyTaskComplete(taskName: string, message: string): Promise<void> {
    await sendNativeNotification({
        title: `${taskName} Complete`,
        body: message,
    });
}

/**
 * Send an error notification
 *
 * @param title Error title
 * @param message Error message
 */
export async function notifyError(title: string, message: string): Promise<void> {
    await sendNativeNotification({
        title: `Error: ${title}`,
        body: message,
        icon: "error",
    });
}

/**
 * Send an update notification
 *
 * @param version New version available
 * @param message Update message
 */
export async function notifyUpdate(version: string, message: string): Promise<void> {
    await sendNativeNotification({
        title: `Update Available: v${version}`,
        body: message,
    });
}
