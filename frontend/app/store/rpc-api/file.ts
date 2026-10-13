// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// File and remote-file operations, plus the editor file-tree commands. Split
// from the original hand-maintained rpc-api file.

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

import type { CreateScratchFileReq } from "@/types/rpc/CreateScratchFileReq";
import type { CreateScratchFileResult } from "@/types/rpc/CreateScratchFileResult";

// The LSP wire shapes are GENERATED from their Rust definitions by ts-rs.
// They used to be function-local `Cmd` structs inside each handler, which is
// exactly why the frontend restated all three by hand: a type declared inside
// a closure cannot be named, let alone generated from.
export type { LspSendReq } from "@/types/rpc/LspSendReq";
export type { LspStartReq } from "@/types/rpc/LspStartReq";
export type { LspStartResult } from "@/types/rpc/LspStartResult";
export type { LspStopReq } from "@/types/rpc/LspStopReq";

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

// The four watcher request shapes, GENERATED like the rest of this file.
export type { UnwatchMediaDirReq } from "@/types/rpc/UnwatchMediaDirReq";
export type { WatchEditorFileReq } from "@/types/rpc/WatchEditorFileReq";
export type { WatchMediaDirReq } from "@/types/rpc/WatchMediaDirReq";

export const FileApi = {
    FileAppendCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("fileappend", data, opts);
    },

    FileJoinCommand(client: RpcClient, data: string[], opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("filejoin", data, opts);
    },

    // Creates a scratch buffer file in ~/.agentmux/cache/scratch/. Returns the backing path + scratch_id.
    // Spec: docs/specs/SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md
    CreateScratchFileCommand(
        client: RpcClient,
        data: CreateScratchFileReq = {},
        opts?: RpcOpts
    ): Promise<CreateScratchFileResult> {
        return client.rpcCall("createscratchfile", data, opts);
    },
};
