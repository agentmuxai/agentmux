import Database from "better-sqlite3";
import { randomUUID } from "crypto";

export interface Message {
  id: string;
  from_agent: string;
  to_agent: string;
  text: string;
  priority: "low" | "normal" | "high" | "urgent";
  timestamp: string;
  read: boolean;
}

export interface Agent {
  id: string;
  last_seen: string;
  messages_sent: number;
}

export class MessageStore {
  private db: Database.Database;

  constructor(dbPath: string = "/data/agentmux.db") {
    this.db = new Database(dbPath);
    this.init();
  }

  private init() {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS messages (
        id TEXT PRIMARY KEY,
        from_agent TEXT NOT NULL,
        to_agent TEXT NOT NULL,
        text TEXT NOT NULL,
        priority TEXT DEFAULT 'normal',
        timestamp TEXT NOT NULL,
        read INTEGER DEFAULT 0
      );
      CREATE INDEX IF NOT EXISTS idx_to_agent ON messages(to_agent);
      CREATE INDEX IF NOT EXISTS idx_timestamp ON messages(timestamp);
    `);
  }

  sendMessage(from: string, to: string, text: string, priority: string = "normal"): Message {
    const id = `msg-${Date.now()}-${randomUUID().slice(0, 8)}`;
    const timestamp = new Date().toISOString();

    this.db.prepare(`
      INSERT INTO messages (id, from_agent, to_agent, text, priority, timestamp, read)
      VALUES (?, ?, ?, ?, ?, ?, 0)
    `).run(id, from, to, text, priority, timestamp);

    return { id, from_agent: from, to_agent: to, text, priority: priority as Message["priority"], timestamp, read: false };
  }

  readMessages(agentId: string, unreadOnly: boolean = true, limit: number = 10, markAsRead: boolean = true): Message[] {
    const query = unreadOnly
      ? `SELECT * FROM messages WHERE (to_agent = ? OR to_agent = '*') AND read = 0 ORDER BY timestamp DESC LIMIT ?`
      : `SELECT * FROM messages WHERE (to_agent = ? OR to_agent = '*') ORDER BY timestamp DESC LIMIT ?`;

    const messages = this.db.prepare(query).all(agentId, limit) as any[];

    if (markAsRead && messages.length > 0) {
      const ids = messages.map(m => m.id);
      const placeholders = ids.map(() => "?").join(",");
      this.db.prepare(`UPDATE messages SET read = 1 WHERE id IN (${placeholders})`).run(...ids);
    }

    return messages.map(m => ({
      ...m,
      read: Boolean(m.read),
      priority: m.priority as Message["priority"]
    }));
  }

  listAgents(): Agent[] {
    const rows = this.db.prepare(`
      SELECT from_agent as id, MAX(timestamp) as last_seen, COUNT(*) as messages_sent
      FROM messages
      GROUP BY from_agent
      ORDER BY last_seen DESC
    `).all() as Agent[];

    return rows;
  }

  deleteMessages(agentId: string, messageIds: string[]): { deleted: string[]; errors: { id: string; error: string }[] } {
    const deleted: string[] = [];
    const errors: { id: string; error: string }[] = [];

    for (const id of messageIds) {
      const msg = this.db.prepare(`SELECT * FROM messages WHERE id = ?`).get(id) as any;
      if (!msg) {
        errors.push({ id, error: "Message not found" });
        continue;
      }
      if (msg.to_agent !== agentId && msg.from_agent !== agentId && msg.to_agent !== "*") {
        errors.push({ id, error: "Not authorized" });
        continue;
      }
      this.db.prepare(`DELETE FROM messages WHERE id = ?`).run(id);
      deleted.push(id);
    }

    return { deleted, errors };
  }

  getStats() {
    const totalMessages = this.db.prepare(`SELECT COUNT(*) as count FROM messages`).get() as { count: number };
    const unreadMessages = this.db.prepare(`SELECT COUNT(*) as count FROM messages WHERE read = 0`).get() as { count: number };
    const uniqueAgents = this.db.prepare(`SELECT COUNT(DISTINCT from_agent) as count FROM messages`).get() as { count: number };

    return {
      total_messages: totalMessages.count,
      unread_messages: unreadMessages.count,
      unique_agents: uniqueAgents.count
    };
  }

  cleanup(maxAgeHours: number = 24) {
    const cutoff = new Date(Date.now() - maxAgeHours * 60 * 60 * 1000).toISOString();
    const result = this.db.prepare(`DELETE FROM messages WHERE timestamp < ?`).run(cutoff);
    return result.changes;
  }
}
