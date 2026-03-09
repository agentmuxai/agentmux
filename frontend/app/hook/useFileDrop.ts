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

    const onDragOver = React.useCallback(
        (e: React.DragEvent) => {
            e.preventDefault();
            e.stopPropagation();
            if (hasFilesDragged(e.dataTransfer) && !isDragOver) {
                setIsDragOver(true);
            }
        },
        [isDragOver]
    );

    const onDragEnter = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (hasFilesDragged(e.dataTransfer)) {
            setIsDragOver(true);
        }
    }, []);

    const onDragLeave = React.useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
        const x = e.clientX;
        const y = e.clientY;
        if (x <= rect.left || x >= rect.right || y <= rect.top || y >= rect.bottom) {
            setIsDragOver(false);
        }
    }, []);

    const onDrop = React.useCallback(
        (e: React.DragEvent) => {
            e.preventDefault();
            e.stopPropagation();
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
