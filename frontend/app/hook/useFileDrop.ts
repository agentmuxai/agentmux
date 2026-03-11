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
//
// KEY DESIGN NOTE: elementRef is passed in by the caller (already attached to the DOM via
// React's ref prop at mount time). We do NOT rely on HTML5 onDragEnter to populate it,
// because in WebView2 with dragDropEnabled:true, OS file drags use the native COM/IDropTarget
// pipeline and HTML5 drag events (dragenter, dragover, dragleave) never fire.
function useFileDrop(
    onFilesDropped: (paths: string[]) => void,
    elementRef: React.RefObject<HTMLElement>
): FileDropResult {
    const [isDragOver, setIsDragOver] = React.useState(false);
    const dragCounter = React.useRef(0);
    const onFilesDroppedRef = React.useRef(onFilesDropped);
    onFilesDroppedRef.current = onFilesDropped;

    // Subscribe once to Tauri's window-level drag-drop event.
    // Use a ref for the callback to avoid re-subscribing on every render.
    React.useEffect(() => {
        console.log("[dnd-debug] useFileDrop: subscribing to Tauri onDragDropEvent");
        let unlistenFn: (() => void) | null = null;
        let cancelled = false; // guard against unlisten resolving after unmount

        getCurrentWebview()
            .onDragDropEvent((event) => {
                const type = event.payload.type;
                const pos = (event.payload as any).position as { x: number; y: number } | undefined;

                const isOverElement = (): boolean => {
                    if (!pos) {
                        console.log("[dnd-debug] isOverElement: no position in event payload");
                        return false;
                    }
                    if (!elementRef.current) {
                        console.log("[dnd-debug] isOverElement: elementRef.current is null — ref not attached");
                        return false;
                    }
                    const rect = elementRef.current.getBoundingClientRect();
                    const inside =
                        pos.x >= rect.left &&
                        pos.x <= rect.right &&
                        pos.y >= rect.top &&
                        pos.y <= rect.bottom;
                    console.log(
                        `[dnd-debug] isOverElement: pos=(${Math.round(pos.x)},${Math.round(pos.y)}) ` +
                        `rect=(${Math.round(rect.left)},${Math.round(rect.top)},${Math.round(rect.right)},${Math.round(rect.bottom)}) ` +
                        `inside=${inside}`
                    );
                    return inside;
                };

                console.log(`[dnd-debug] Tauri onDragDropEvent type=${type}`, pos ? `pos=(${Math.round(pos.x)},${Math.round(pos.y)})` : "no-pos");

                if (type === "enter" || type === "over") {
                    const inside = isOverElement();
                    console.log(`[dnd-debug] ${type}: setting isDragOver=${inside}`);
                    setIsDragOver(inside);
                } else if (type === "drop") {
                    const paths: string[] = (event.payload as any).paths ?? [];
                    const inside = isOverElement();
                    console.log(`[dnd-debug] drop: paths=${JSON.stringify(paths)}, inside=${inside}`);
                    if (inside && paths.length > 0) {
                        console.log("[dnd-debug] drop: ✓ calling onFilesDropped with", paths);
                        onFilesDroppedRef.current(paths);
                    } else if (!inside) {
                        console.log("[dnd-debug] drop: ignoring — not over this element");
                    } else {
                        console.log("[dnd-debug] drop: ignoring — paths array is empty");
                    }
                    setIsDragOver(false);
                    dragCounter.current = 0;
                } else if (type === "leave") {
                    console.log("[dnd-debug] leave: clearing isDragOver");
                    setIsDragOver(false);
                    dragCounter.current = 0;
                }
            })
            .then((fn) => {
                if (cancelled) {
                    // Component unmounted before the promise resolved — unlisten immediately
                    console.log("[dnd-debug] useFileDrop: component already unmounted, unlistening immediately");
                    fn();
                } else {
                    unlistenFn = fn;
                    console.log("[dnd-debug] useFileDrop: ✓ subscribed to onDragDropEvent");
                }
            })
            .catch((err) => {
                console.error("[dnd-debug] useFileDrop: ✗ failed to subscribe to onDragDropEvent:", err);
            });

        return () => {
            cancelled = true;
            console.log("[dnd-debug] useFileDrop: unsubscribing");
            unlistenFn?.();
        };
    }, []); // subscribe once — callback accessed via ref

    // HTML5 drag events: kept as fallback for isDragOver on platforms where
    // HTML5 events do fire alongside Tauri events. Also needed to call
    // e.preventDefault() so the browser doesn't try to open the file.
    // NOTE: these may NOT fire in WebView2 for OS file drags — that's expected.
    const onDragOver = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
    }, []);

    const onDragEnter = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (e.dataTransfer.types.includes("Files")) {
            dragCounter.current += 1;
            console.log(`[dnd-debug] HTML5 onDragEnter fired (counter=${dragCounter.current}) — note: may not fire in WebView2`);
            setIsDragOver(true);
        }
    }, []);

    const onDragLeave = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        dragCounter.current -= 1;
        console.log(`[dnd-debug] HTML5 onDragLeave fired (counter=${dragCounter.current})`);
        if (dragCounter.current <= 0) {
            dragCounter.current = 0;
            setIsDragOver(false);
        }
    }, []);

    const onDrop = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        console.log("[dnd-debug] HTML5 onDrop fired — Tauri handler does the actual file work");
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
