---
'agentmux-srv': patch
---

feat(blockfile): output.idx byte-offset index for O(1) line seek

Appends a companion `output.idx` file alongside every NDJSON output file.
Each entry is a u64-LE value recording the byte offset where line N starts.
`blockfile:read_range` checks for this index first; if present it seeks
directly to the requested line range without loading the full file.

Existing sessions without an index fall back to the previous full-scan path.
