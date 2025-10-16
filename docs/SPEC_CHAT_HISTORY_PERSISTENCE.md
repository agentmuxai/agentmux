# AgentMux Chat History Persistence Specification

**Version:** 1.0
**Date:** 2025-10-15
**Status:** Draft

---

## Executive Summary

Implement **persistent chat history** for Claude CLI conversations in AgentMux Desktop, allowing users to:
- **Resume conversations** across app restarts
- **Search previous interactions** with agents
- **Export conversation logs** for reference
- **Manage storage** (clear old chats, set retention policies)

**Core Value:** Never lose context. Pick up where you left off.

---

## 1. Problem Statement

### Current Behavior
- **Terminal clears** on app restart
- **No conversation history** preserved
- **Lost context** when switching agents
- **Can't reference** previous exchanges

### User Pain Points
1. Restart app → lose all previous conversation
2. Switch to different agent → can't review what Agent1 said
3. Want to reference earlier solution → no search capability
4. Need to share conversation → no export function

---

## 2. Requirements

### 2.1 Functional Requirements

#### Must Have (v0.4.0)
- **F1.1** Persist all Claude CLI input/output
- **F1.2** Restore conversation on app restart
- **F1.3** Associate history with agent instance
- **F1.4** Display history in terminal on load
- **F1.5** Clear history command (per agent or global)

#### Should Have (v0.5.0)
- **F2.1** Search across conversation history
- **F2.2** Export conversation to markdown/txt
- **F2.3** Retention policy (auto-delete old chats)
- **F2.4** History viewer UI (separate from terminal)

#### Nice to Have (v0.6.0)
- **F3.1** Conversation bookmarks/tags
- **F3.2** Diff view between conversations
- **F3.3** Share conversations via link
- **F3.4** Sync history across devices

---

### 2.2 Non-Functional Requirements

#### Performance
- **NFR1** History load time: < 200ms for 10K messages
- **NFR2** Search response time: < 100ms for 1K messages
- **NFR3** Storage overhead: < 1MB per 1K messages

#### Storage
- **NFR4** SQLite database for structured storage
- **NFR5** Indexed by timestamp, agent ID, message type
- **NFR6** Compression for large conversations (> 1MB)

#### Privacy
- **NFR7** Local-only storage (no cloud upload)
- **NFR8** Optional encryption for sensitive conversations
- **NFR9** User-controlled deletion (GDPR compliance)

---

## 3. Data Model

### 3.1 Database Schema

```sql
-- Main table: conversations
CREATE TABLE conversations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  agent_id TEXT NOT NULL,
  agent_name TEXT,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  status TEXT DEFAULT 'active', -- active, archived, deleted
  metadata JSON -- user tags, bookmarks, etc.
);

-- Table: messages
CREATE TABLE messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  conversation_id INTEGER NOT NULL,
  timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  direction TEXT NOT NULL, -- 'input' or 'output'
  content TEXT NOT NULL,
  content_type TEXT DEFAULT 'text', -- text, ansi, markdown
  metadata JSON, -- tool calls, file operations, etc.
  FOREIGN KEY (conversation_id) REFERENCES conversations(id)
);

-- Index for fast lookups
CREATE INDEX idx_conversation_agent ON conversations(agent_id);
CREATE INDEX idx_messages_conversation ON messages(conversation_id);
CREATE INDEX idx_messages_timestamp ON messages(timestamp);
CREATE INDEX idx_messages_content ON messages(content); -- FTS5 for search
```

### 3.2 TypeScript Interfaces

```typescript
interface Conversation {
  id: number;
  agentId: string;
  agentName?: string;
  createdAt: Date;
  updatedAt: Date;
  status: 'active' | 'archived' | 'deleted';
  metadata?: {
    tags?: string[];
    bookmarks?: number[];
    notes?: string;
  };
}

interface Message {
  id: number;
  conversationId: number;
  timestamp: Date;
  direction: 'input' | 'output';
  content: string;
  contentType: 'text' | 'ansi' | 'markdown';
  metadata?: {
    toolCalls?: ToolCall[];
    fileOps?: FileOperation[];
    exitCode?: number;
  };
}

interface ToolCall {
  tool: string;
  args: Record<string, any>;
  result?: any;
}

interface FileOperation {
  type: 'read' | 'write' | 'edit';
  path: string;
  linesChanged?: number;
}
```

---

## 4. Implementation Architecture

### 4.1 Component Structure

```
History Module
├── HistoryService.rs (Rust backend)
│   ├── Database connection (rusqlite)
│   ├── CRUD operations
│   ├── Search/filter queries
│   └── Export functions
├── HistoryManager.ts (Frontend service)
│   ├── Cache management
│   ├── UI state sync
│   └── Event handlers
└── Tauri Commands
    ├── save_message()
    ├── load_conversation()
    ├── search_history()
    ├── export_conversation()
    └── clear_history()
```

### 4.2 Data Flow

```
1. User types in terminal
   ↓
2. Input captured by EmbeddedTerminal.tsx
   ↓
3. Tauri command: save_message(input)
   ↓
4. HistoryService.rs writes to SQLite
   ↓
5. Claude processes and responds
   ↓
6. Output captured from PTY
   ↓
7. Tauri command: save_message(output)
   ↓
8. HistoryService.rs writes to SQLite
```

**On app restart:**
```
1. App launches
   ↓
2. Tauri command: load_conversation(agent_id)
   ↓
3. HistoryService.rs queries SQLite
   ↓
4. Messages loaded into terminal buffer
   ↓
5. Terminal displays conversation history
```

---

## 5. Storage Strategy

### 5.1 Database Location

**Path:** `~/.agentmux/history.db` (or `%APPDATA%\agentmux\history.db` on Windows)

**Rationale:**
- User-writable location
- Survives app uninstall
- Easy to backup/restore
- Platform-agnostic

### 5.2 Storage Limits

| Metric | Default Limit | Configurable? |
|--------|---------------|---------------|
| Max messages per conversation | 10,000 | ✅ |
| Max conversation age | 90 days | ✅ |
| Max total DB size | 500 MB | ✅ |
| Auto-archive threshold | 30 days | ✅ |

**Auto-cleanup policy:**
- Archive conversations > 30 days old
- Delete archived conversations > 90 days old
- User can override via settings

---

## 6. User Interface

### 6.1 History Restoration

**On terminal load:**
```
┌─────────────────────────────────────────┐
│ Claude Agent 1                          │
│                                         │
│ ─── Previous conversation (2h ago) ───  │
│                                         │
│ > help me deploy pulse                  │
│ I'll help you deploy pulse...           │
│                                         │
│ > cd pulse && npm run deploy            │
│ Deploying to production...              │
│                                         │
│ ─────── Session resumed ───────         │
│                                         │
│ > █                                     │
└─────────────────────────────────────────┘
```

**Visual indicator** shows where old conversation ends and new session begins.

---

### 6.2 History Viewer Modal

**Accessed via hamburger menu:**
```
┌──────────────────────────────────────────────┐
│  Conversation History                   [X]  │
├──────────────────────────────────────────────┤
│  Search: [________________] [🔍]             │
│                                              │
│  ┌──────────────────────────────────────┐   │
│  │ Agent1 - Today at 2:30 PM            │   │
│  │ "help me deploy pulse"               │   │
│  │ 23 messages | [Open] [Export]        │   │
│  ├──────────────────────────────────────┤   │
│  │ Agent2 - Yesterday at 4:15 PM        │   │
│  │ "fix authentication bug"             │   │
│  │ 15 messages | [Open] [Export]        │   │
│  └──────────────────────────────────────┘   │
│                                              │
│  [Clear All History]                         │
└──────────────────────────────────────────────┘
```

---

### 6.3 Export Options

**Export dialog:**
```
┌─────────────────────────────────────┐
│  Export Conversation           [X]  │
├─────────────────────────────────────┤
│                                     │
│  Format:                            │
│  ○ Markdown (.md)                   │
│  ○ Plain Text (.txt)                │
│  ○ JSON (.json)                     │
│                                     │
│  Include:                           │
│  ☑ Timestamps                       │
│  ☑ Agent metadata                   │
│  ☐ ANSI colors                      │
│                                     │
│  [Cancel]  [Export]                 │
└─────────────────────────────────────┘
```

**Markdown output example:**
```markdown
# Conversation with Agent1
**Date:** 2025-10-15 14:30
**Agent:** agent-12345-1760559248

## Messages

**[14:30:15] User:**
> help me deploy pulse

**[14:30:18] Agent1:**
I'll help you deploy pulse. First, let me check the deployment scripts...

**[14:30:22] User:**
> cd pulse && npm run deploy

**[14:30:25] Agent1:**
Deploying to production...
✓ Build completed
✓ Tests passed
✓ Deployed to AWS
```

---

## 7. Implementation Plan

### Phase 1: Core Persistence (Week 1-2)
**Goal:** Save and restore basic conversation history

**Tasks:**
- [ ] Create SQLite database schema
- [ ] Implement HistoryService.rs (Rust)
- [ ] Add Tauri commands (save/load messages)
- [ ] Hook into PTY input/output capture
- [ ] Test: Restart app and see history

**Deliverables:**
- Working SQLite database
- Messages persisted across restarts
- Basic history display in terminal

---

### Phase 2: History Viewer UI (Week 3)
**Goal:** Allow users to browse and search history

**Tasks:**
- [ ] Create HistoryViewerModal.tsx component
- [ ] Implement search functionality
- [ ] Add conversation list UI
- [ ] Add hamburger menu entry
- [ ] Test: Search for specific message

**Deliverables:**
- History viewer modal
- Search working across all conversations
- Menu integration

---

### Phase 3: Export & Management (Week 4)
**Goal:** Export conversations and manage storage

**Tasks:**
- [ ] Implement export to Markdown/TXT/JSON
- [ ] Add "Clear History" command
- [ ] Implement retention policies
- [ ] Add settings for history limits
- [ ] Test: Export and verify format

**Deliverables:**
- Export dialog with format options
- Clear history functionality
- Auto-cleanup based on retention policy

---

### Phase 4: Advanced Features (Week 5-6)
**Goal:** Search, tags, and polish

**Tasks:**
- [ ] Full-text search (SQLite FTS5)
- [ ] Conversation tagging/bookmarking
- [ ] Date range filtering
- [ ] Performance optimization
- [ ] Test: Search 10K messages in < 100ms

**Deliverables:**
- Fast search across all history
- Tagging system
- Performance benchmarks met

---

## 8. Technical Considerations

### 8.1 Rust Backend (HistoryService.rs)

```rust
use rusqlite::{Connection, params};

pub struct HistoryService {
    conn: Connection,
}

impl HistoryService {
    pub fn new(db_path: &str) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;
        Self::init_schema(&conn)?;
        Ok(HistoryService { conn })
    }

    fn init_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(())
    }

    pub fn save_message(
        &self,
        conversation_id: i64,
        direction: &str,
        content: &str,
    ) -> Result<i64, rusqlite::Error> {
        conn.execute(
            "INSERT INTO messages (conversation_id, direction, content) VALUES (?1, ?2, ?3)",
            params![conversation_id, direction, content],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn load_conversation(
        &self,
        agent_id: &str,
    ) -> Result<Vec<Message>, rusqlite::Error> {
        // Query messages for agent's active conversation
        // ...
    }
}
```

---

### 8.2 Frontend Integration (EmbeddedTerminal.tsx)

```typescript
// Capture user input
const handleInput = async (input: string) => {
  // Save to history
  await invoke('save_message', {
    conversationId: currentConversation.id,
    direction: 'input',
    content: input,
  });

  // Send to Claude
  await sendToClaude(input);
};

// Capture Claude output
const handleOutput = async (output: string) => {
  // Save to history
  await invoke('save_message', {
    conversationId: currentConversation.id,
    direction: 'output',
    content: output,
  });

  // Display in terminal
  terminal.write(output);
};

// Load history on component mount
useEffect(() => {
  const loadHistory = async () => {
    const messages = await invoke<Message[]>('load_conversation', {
      agentId: instance.agentId,
    });

    // Replay messages into terminal
    messages.forEach((msg) => {
      const prefix = msg.direction === 'input' ? '> ' : '';
      terminal.writeln(prefix + msg.content);
    });

    // Show separator
    terminal.writeln('\n─────── Session resumed ───────\n');
  };

  loadHistory();
}, [instance.agentId]);
```

---

## 9. Edge Cases

### 9.1 Large Conversations
**Problem:** Conversation with 10,000+ messages takes too long to load

**Solution:**
- Paginate history loading (load last 100 messages initially)
- "Load more" button to fetch older messages
- Virtual scrolling for large message lists

---

### 9.2 Concurrent Agents
**Problem:** Multiple agents writing to same conversation

**Solution:**
- Each agent gets own conversation ID
- Agent ID embedded in conversation metadata
- Clear separation in UI ("Agent1 session", "Agent2 session")

---

### 9.3 Database Corruption
**Problem:** SQLite database becomes corrupted

**Solution:**
- Periodic integrity checks (`PRAGMA integrity_check`)
- Automatic backup every 24 hours
- Recovery tool to rebuild from message logs
- User notification if corruption detected

---

### 9.4 Storage Limits
**Problem:** Database grows to 1GB+

**Solution:**
- Warning at 500MB threshold
- Auto-archive old conversations
- User prompt to delete archived conversations
- Export to external file before deletion

---

## 10. Testing Strategy

### 10.1 Unit Tests
- [ ] Save message to database
- [ ] Load conversation from database
- [ ] Search messages (exact match)
- [ ] Search messages (fuzzy match)
- [ ] Export to Markdown format
- [ ] Clear history (single conversation)
- [ ] Clear history (all conversations)

---

### 10.2 Integration Tests
- [ ] Full conversation flow (input → save → restart → load)
- [ ] Multiple agents, separate histories
- [ ] Export after 1000 messages
- [ ] Search across 10K messages
- [ ] Auto-cleanup after retention period

---

### 10.3 Performance Tests
- [ ] Load 10K messages in < 200ms
- [ ] Search 10K messages in < 100ms
- [ ] Save message in < 10ms
- [ ] Export 1K messages in < 500ms

---

## 11. Privacy & Security

### 11.1 Data Sensitivity
**Assumption:** Conversations may contain:
- Proprietary code
- API keys (accidentally pasted)
- Sensitive business logic
- Personal information

**Mitigation:**
- Local-only storage (no cloud upload)
- Optional database encryption (SQLCipher)
- Clear history command (immediate deletion)
- Retention policies (auto-delete old data)

---

### 11.2 Encryption (Optional Feature)

**Implementation:**
```rust
use sqlcipher::Connection;

// Encrypted database with user password
let conn = Connection::open_encrypted(
    db_path,
    user_password,
)?;
```

**UI:**
```
┌─────────────────────────────────────┐
│  Enable History Encryption     [X]  │
├─────────────────────────────────────┤
│                                     │
│  Protect your conversation history  │
│  with a password.                   │
│                                     │
│  Password: [___________________]    │
│  Confirm:  [___________________]    │
│                                     │
│  ⚠️  If you forget your password,   │
│     your history cannot be          │
│     recovered.                      │
│                                     │
│  [Cancel]  [Enable Encryption]      │
└─────────────────────────────────────┘
```

---

## 12. Success Metrics

### Quantitative
- **History load time:** < 200ms (10K messages)
- **Search time:** < 100ms (1K messages)
- **Export time:** < 500ms (1K messages)
- **Storage efficiency:** < 1KB per message

### Qualitative
- Users report "never losing context"
- Positive feedback on history search
- Export feature used regularly
- Zero complaints about performance

---

## 13. Future Enhancements

### Post v0.5.0
- **Conversation branching** (fork at specific message)
- **Diff view** (compare two conversations)
- **Collaborative history** (share with team)
- **Cloud sync** (optional, encrypted)
- **AI-powered search** (semantic search with embeddings)
- **Conversation summaries** (auto-generated TL;DR)

---

## 14. Open Questions

1. **Retention policy** - Default 90 days, or infinite?
2. **Encryption** - On by default, or opt-in?
3. **Export format** - Markdown, or JSON as default?
4. **Search scope** - Current agent only, or all agents?
5. **UI placement** - History viewer in menu, or always-visible panel?

---

## Appendix A: Database Size Estimates

**Assumptions:**
- Average message: 500 bytes
- Average conversation: 100 messages
- Active user: 10 conversations/day

**Storage projections:**
```
1 message:        ~500 bytes
1 conversation:   ~50 KB
1 day:            ~500 KB
1 month:          ~15 MB
1 year:           ~180 MB
```

**With compression (gzip):**
```
1 year:           ~60 MB (67% reduction)
```

**Conclusion:** 500 MB limit allows ~8 years of history uncompressed, or ~24 years compressed.

---

## Appendix B: SQL Schema (Complete)

```sql
-- Full schema with all tables and indexes

CREATE TABLE conversations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  agent_id TEXT NOT NULL,
  agent_name TEXT,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  status TEXT DEFAULT 'active',
  metadata JSON
);

CREATE TABLE messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  conversation_id INTEGER NOT NULL,
  timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  direction TEXT NOT NULL,
  content TEXT NOT NULL,
  content_type TEXT DEFAULT 'text',
  metadata JSON,
  FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);

-- Full-text search virtual table
CREATE VIRTUAL TABLE messages_fts USING fts5(
  content,
  content=messages,
  content_rowid=id
);

-- Triggers to keep FTS in sync
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
  DELETE FROM messages_fts WHERE rowid = old.id;
END;

CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
  UPDATE messages_fts SET content = new.content WHERE rowid = new.id;
END;

-- Indexes
CREATE INDEX idx_conversation_agent ON conversations(agent_id);
CREATE INDEX idx_conversation_status ON conversations(status);
CREATE INDEX idx_messages_conversation ON messages(conversation_id);
CREATE INDEX idx_messages_timestamp ON messages(timestamp);
CREATE INDEX idx_messages_direction ON messages(direction);
```

---

**Next Steps:**
1. Review and approve this spec
2. Set up SQLite integration in Rust
3. Begin Phase 1 implementation
4. Schedule weekly progress reviews
