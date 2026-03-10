// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getCurrentWebview } from "@tauri-apps/api/webview";
import * as React from "react";

interface FileDropResult {
    isDragOver: boolean;
    handlers: {
        onDragOver: (e: React.DragEvent) => void;
        onDragEnter: (e: React.DragEvent) => void;
        onDragLeave: (e: React.DragEvent) => void;
        onDrop: (e: React.DragEvent) => void;
    };
}

// useFileDrop uses Tauri's onDragDropEvent (requires dragDropEnabled: true in tauri.conf.json)
// which provides real OS filesystem paths. Element targeting uses getBoundingClientRect
// against the drag position since Tauri fires window-level events, not element-level.
// HTML5 drag events are also handled as a fallback for isDragOver visual state.
function useFileDrop(onFilesDropped: (paths: string[]) => void): FileDropResult {
    const [isDragOver, setIsDragOver] = React.useState(false);
    const elementRef = React.useRef<HTMLElement | null>(null);
    const dragCounter = React.useRef(0);
    const onFilesDroppedRef = React.useRef(onFilesDropped);
    onFilesDroppedRef.current = onFilesDropped;

    // Subscribe once to Tauri's window-level drag-drop event.
    // Use a ref for the callback to avoid re-subscribing on every render.
    React.useEffect(() => {
        let unlisten: (() => void) | null = null;

        getCurrentWebview()
            .onDragDropEvent((event) => {
                const type = event.payload.type;
                const pos = (event.payload as any).position as { x: number; y: number } | undefined;

                const isOverElement = (): boolean => {
                    if (!pos || !elementRef.current) return false;
                    const rect = elementRef.current.getBoundingClientRect();
                    return (
                        pos.x >= rect.left &&
                        pos.x <= rect.right &&
                        pos.y >= rect.top &&
                        pos.y <= rect.bottom
                    );
                };

                if (type === "enter" || type === "over") {
                    if (isOverElement()) {
                        setIsDragOver(true);
                    } else {
                        setIsDragOver(false);
                    }
                } else if (type === "drop") {
                    const paths: string[] = (event.payload as any).paths ?? [];
                    console.log("[dnd-debug] tauri drop event, paths:", paths, "over element:", isOverElement());
                    if (isOverElement() && paths.length > 0) {
                        onFilesDroppedRef.current(paths);
                    }
                    setIsDragOver(false);
                    dragCounter.current = 0;
                } else if (type === "leave") {
                    setIsDragOver(false);
                    dragCounter.current = 0;
                }
            })
            .then((fn) => {
                unlisten = fn;
            });

        return () => {
            unlisten?.();
        };
    }, []); // subscribe once — callback accessed via ref

    // HTML5 drag events: used as a fallback signal for isDragOver in case Tauri
    // 'enter'/'over' events don't arrive (e.g. on some platforms).
    // Also needed to call e.preventDefault() so the browser doesn't open the file.
    const onDragOver = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
    }, []);

    const onDragEnter = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (e.dataTransfer.types.includes("Files")) {
            dragCounter.current += 1;
            setIsDragOver(true);
            // Store a ref to this element for Tauri position checks
            elementRef.current = e.currentTarget as HTMLElement;
        }
    }, []);

    const onDragLeave = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        dragCounter.current -= 1;
        if (dragCounter.current <= 0) {
            dragCounter.current = 0;
            setIsDragOver(false);
        }
    }, []);

    const onDrop = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        // Reset overlay — Tauri event handler does the actual file processing.
        // This ensures the overlay clears even if the Tauri event fires slightly later.
        dragCounter.current = 0;
        setIsDragOver(false);
    }, []);

    return {
        isDragOver,
        handlers: { onDragOver, onDragEnter, onDragLeave, onDrop },
    };
}

export { useFileDrop };
export type { FileDropResult };
