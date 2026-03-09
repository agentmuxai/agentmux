// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

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

function hasFilesDragged(dataTransfer: DataTransfer): boolean {
    return dataTransfer.types.includes("Files");
}

function useFileDrop(onFilesDropped: (files: File[]) => void): FileDropResult {
    const [isDragOver, setIsDragOver] = React.useState(false);
    // Counter tracks nested enter/leave events from child elements.
    // dragLeave fires for each child element crossed — we only clear when counter
    // reaches 0 (i.e., the drag has truly left the entire drop zone).
    const dragCounter = React.useRef(0);

    // Reset counter + state when component unmounts or drag ends externally
    React.useEffect(() => {
        const handleDragEnd = () => {
            dragCounter.current = 0;
            setIsDragOver(false);
        };
        document.addEventListener("dragend", handleDragEnd);
        document.addEventListener("drop", handleDragEnd);
        return () => {
            document.removeEventListener("dragend", handleDragEnd);
            document.removeEventListener("drop", handleDragEnd);
        };
    }, []);

    const onDragOver = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
    }, []);

    const onDragEnter = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (hasFilesDragged(e.dataTransfer)) {
            dragCounter.current += 1;
            setIsDragOver(true);
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

    const onDrop = React.useCallback(
        (e: React.DragEvent) => {
            e.preventDefault();
            e.stopPropagation();
            dragCounter.current = 0;
            setIsDragOver(false);
            const files = Array.from(e.dataTransfer.files);
            if (files.length > 0) {
                onFilesDropped(files);
            }
        },
        [onFilesDropped]
    );

    return {
        isDragOver,
        handlers: { onDragOver, onDragEnter, onDragLeave, onDrop },
    };
}

export { useFileDrop };
export type { FileDropResult };
