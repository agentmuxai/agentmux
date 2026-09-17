// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// File and remote-file operations, plus the editor file-tree commands. Split
// from the original hand-maintained rpc-api.ts.

import { RpcClient } from "../rpc-client";

// The editor file-tree mutation shapes are GENERATED from their Rust
// definitions by ts-rs. Each of them used to be a `struct Cmd` declared
// inside its handler's own closure, with the response built by an inline
// `json!({..})` -- neither form can be named, so neither could be generated
// from, which is why every one of these was restated by hand here.
export type { CreateEditorDirReq } from "@/types/rpc/CreateEditorDirReq";
export type { CreateEditorDirResult } from "@/types/rpc/CreateEditorDirResult";
export type { CreateEditorFileReq } from "@/types/rpc/CreateEditorFileReq";
export type { CreateEditorFileResult } from "@/types/rpc/CreateEditorFileResult";
export type { CreateScratchFileReq } from "@/types/rpc/CreateScratchFileReq";
export type { CreateScratchFileResult } from "@/types/rpc/CreateScratchFileResult";
export type { DeleteEditorFileReq } from "@/types/rpc/DeleteEditorFileReq";
export type { MoveScratchFileReq } from "@/types/rpc/MoveScratchFileReq";
export type { MoveScratchFileResult } from "@/types/rpc/MoveScratchFileResult";
export type { OpenInShellReq } from "@/types/rpc/OpenInShellReq";
export type { RenameEditorFileReq } from "@/types/rpc/RenameEditorFileReq";
export type { RenameEditorFileResult } from "@/types/rpc/RenameEditorFileResult";

import type { CreateEditorDirReq } from "@/types/rpc/CreateEditorDirReq";
import type { CreateEditorDirResult } from "@/types/rpc/CreateEditorDirResult";
import type { CreateEditorFileReq } from "@/types/rpc/CreateEditorFileReq";
import type { CreateEditorFileResult } from "@/types/rpc/CreateEditorFileResult";
import type { CreateScratchFileReq } from "@/types/rpc/CreateScratchFileReq";
import type { CreateScratchFileResult } from "@/types/rpc/CreateScratchFileResult";
import type { DeleteEditorFileReq } from "@/types/rpc/DeleteEditorFileReq";
import type { MoveScratchFileReq } from "@/types/rpc/MoveScratchFileReq";
import type { MoveScratchFileResult } from "@/types/rpc/MoveScratchFileResult";
import type { OpenInShellReq } from "@/types/rpc/OpenInShellReq";
import type { RenameEditorFileReq } from "@/types/rpc/RenameEditorFileReq";
import type { RenameEditorFileResult } from "@/types/rpc/RenameEditorFileResult";

// The LSP wire shapes are GENERATED from their Rust definitions by ts-rs.
// They used to be function-local `Cmd` structs inside each handler, which is
// exactly why the frontend restated all three by hand: a type declared inside
// a closure cannot be named, let alone generated from.
export type { LspSendReq } from "@/types/rpc/LspSendReq";
export type { LspStartReq } from "@/types/rpc/LspStartReq";
export type { LspStartResult } from "@/types/rpc/LspStartResult";
export type { LspStopReq } from "@/types/rpc/LspStopReq";

import type { LspSendReq } from "@/types/rpc/LspSendReq";
import type { LspStartReq } from "@/types/rpc/LspStartReq";
import type { LspStartResult } from "@/types/rpc/LspStartResult";
import type { LspStopReq } from "@/types/rpc/LspStopReq";

// The editor read shapes are GENERATED from their Rust definitions by ts-rs,
// same story as the mutation shapes above: closure-local `Cmd` structs and
// inline `json!({..})` responses, with hand-written TS restating both ends.
export type { CommandReadEditorFileData } from "@/types/rpc/CommandReadEditorFileData";
export type { CommandReadEditorFileResult } from "@/types/rpc/CommandReadEditorFileResult";
export type { CommandWriteEditorFileData } from "@/types/rpc/CommandWriteEditorFileData";
export type { DirEntry } from "@/types/rpc/DirEntry";
export type { EditorDrive } from "@/types/rpc/EditorDrive";
export type { EditorRootsReq } from "@/types/rpc/EditorRootsReq";
export type { GetEditorHomeResult } from "@/types/rpc/GetEditorHomeResult";
export type { GetEditorRootsResult } from "@/types/rpc/GetEditorRootsResult";
export type { ListEditorDirReq } from "@/types/rpc/ListEditorDirReq";
export type { ListEditorDirResult } from "@/types/rpc/ListEditorDirResult";

import type { CommandReadEditorFileData } from "@/types/rpc/CommandReadEditorFileData";
import type { CommandReadEditorFileResult } from "@/types/rpc/CommandReadEditorFileResult";
import type { CommandWriteEditorFileData } from "@/types/rpc/CommandWriteEditorFileData";
import type { DirEntry } from "@/types/rpc/DirEntry";
import type { EditorDrive } from "@/types/rpc/EditorDrive";
import type { EditorRootsReq } from "@/types/rpc/EditorRootsReq";
import type { GetEditorHomeResult } from "@/types/rpc/GetEditorHomeResult";
import type { GetEditorRootsResult } from "@/types/rpc/GetEditorRootsResult";
import type { ListEditorDirReq } from "@/types/rpc/ListEditorDirReq";
import type { ListEditorDirResult } from "@/types/rpc/ListEditorDirResult";

// The four watcher request shapes, GENERATED like the rest of this file.
export type { UnwatchMediaDirReq } from "@/types/rpc/UnwatchMediaDirReq";
export type { WatchEditorFileReq } from "@/types/rpc/WatchEditorFileReq";
export type { WatchMediaDirReq } from "@/types/rpc/WatchMediaDirReq";

import type { UnwatchMediaDirReq } from "@/types/rpc/UnwatchMediaDirReq";
import type { WatchEditorFileReq } from "@/types/rpc/WatchEditorFileReq";
import type { WatchMediaDirReq } from "@/types/rpc/WatchMediaDirReq";

export const FileApi = {
    FileAppendCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("fileappend", data, opts);
    },

    FileJoinCommand(client: RpcClient, data: string[], opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("filejoin", data, opts);
    },

    ReadEditorFileCommand(client: RpcClient, data: CommandReadEditorFileData, opts?: RpcOpts): Promise<CommandReadEditorFileResult> {
        return client.rpcCall("readeditorfile", data, opts);
    },

    WriteEditorFileCommand(client: RpcClient, data: CommandWriteEditorFileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("writeeditorfile", data, opts);
    },

    // Start/stop live-reload watching for a (path, block_id) pair. The
    // backend publishes `editor:file_changed` (scoped to `block:<block_id>`)
    // when the path changes on disk. See
    // docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md.
    WatchEditorFileCommand(
        client: RpcClient,
        data: WatchEditorFileReq,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("watcheditorfile", data, opts);
    },

    UnwatchEditorFileCommand(
        client: RpcClient,
        // Same type as the watch: an unwatch that cannot name exactly what the
        // watch named leaks a watcher.
        data: WatchEditorFileReq,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("unwatcheditorfile", data, opts);
    },

    // Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md
    ListEditorDirCommand(
        client: RpcClient,
        data: ListEditorDirReq,
        opts?: RpcOpts,
    ): Promise<ListEditorDirResult> {
        return client.rpcCall("listeditordir", data, opts);
    },

    // Media pane (SPEC_MEDIA_PANE_2026_07_26.md): watch a directory for
    // new/changed files matching `extensions` (lowercase, no dot). Backend
    // publishes `media:file_changed` (scoped to `block:<block_id>`) with
    // `{ path }` when a matching file is created/modified.
    WatchMediaDirCommand(
        client: RpcClient,
        data: WatchMediaDirReq,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("watchmediadir", data, opts);
    },

    UnwatchMediaDirCommand(
        client: RpcClient,
        data: UnwatchMediaDirReq,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("unwatchmediadir", data, opts);
    },

    GetEditorHomeCommand(
        client: RpcClient,
        data: EditorRootsReq = {},
        opts?: RpcOpts,
    ): Promise<GetEditorHomeResult> {
        return client.rpcCall("geteditorhome", data, opts);
    },

    // Returns home + drives/mounts; the editor file-tree renders these as sibling top-level roots.
    GetEditorRootsCommand(
        client: RpcClient,
        data: EditorRootsReq = {},
        opts?: RpcOpts,
    ): Promise<GetEditorRootsResult> {
        return client.rpcCall("geteditorroots", data, opts);
    },

    // Spec: docs/specs/SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md
    OpenInShellCommand(
        client: RpcClient,
        data: OpenInShellReq,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("openinshell", data, opts);
    },

    RenameEditorFileCommand(
        client: RpcClient,
        data: RenameEditorFileReq,
        opts?: RpcOpts,
    ): Promise<RenameEditorFileResult> {
        return client.rpcCall("renameeditorfile", data, opts);
    },

    CreateEditorFileCommand(
        client: RpcClient,
        data: CreateEditorFileReq,
        opts?: RpcOpts,
    ): Promise<CreateEditorFileResult> {
        return client.rpcCall("createeditorfile", data, opts);
    },

    CreateEditorDirCommand(
        client: RpcClient,
        data: CreateEditorDirReq,
        opts?: RpcOpts,
    ): Promise<CreateEditorDirResult> {
        return client.rpcCall("createeditordir", data, opts);
    },

    DeleteEditorFileCommand(
        client: RpcClient,
        data: DeleteEditorFileReq,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("deleteeditorfile", data, opts);
    },

    // Creates a scratch buffer file in ~/.agentmux/cache/scratch/. Returns the backing path + scratch_id.
    // Spec: docs/specs/SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md
    CreateScratchFileCommand(
        client: RpcClient,
        data: CreateScratchFileReq = {},
        opts?: RpcOpts,
    ): Promise<CreateScratchFileResult> {
        return client.rpcCall("createscratchfile", data, opts);
    },

    MoveScratchFileCommand(
        client: RpcClient,
        data: MoveScratchFileReq,
        opts?: RpcOpts,
    ): Promise<MoveScratchFileResult> {
        return client.rpcCall("movescratchfile", data, opts);
    },

    // ── LSP — Phase 1 of SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md ──────
    // Backend is a dumb proxy: lspstart spawns (or attaches to) the
    // server for (workspace, language); lspsend forwards an arbitrary
    // LSP JSON-RPC message to its stdin; lspstop refcount-decrements.
    // Server-pushed notifications arrive via the `lsp:message` WS event.

    LspStartCommand(
        client: RpcClient,
        data: LspStartReq,
        opts?: RpcOpts,
    ): Promise<LspStartResult> {
        return client.rpcCall("lspstart", data, opts);
    },

    LspSendCommand(
        client: RpcClient,
        data: LspSendReq,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("lspsend", data, opts);
    },

    LspStopCommand(
        client: RpcClient,
        data: LspStopReq,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("lspstop", data, opts);
    },
};
