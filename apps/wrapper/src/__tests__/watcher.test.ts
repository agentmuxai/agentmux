import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { MessageWatcher } from '../watcher';
import { Message } from '../types';

describe('MessageWatcher', () => {
  let testMessagesDir: string;
  let messageCallback: jest.Mock;

  beforeEach(() => {
    // Create temporary test directory
    testMessagesDir = path.join(os.tmpdir(), `agentmux-test-${Date.now()}`);
    fs.mkdirSync(testMessagesDir, { recursive: true });

    messageCallback = jest.fn();
  });

  afterEach(() => {
    // Clean up test directory
    if (fs.existsSync(testMessagesDir)) {
      fs.rmSync(testMessagesDir, { recursive: true, force: true });
    }
  });

  describe('message routing', () => {
    it('should trigger callback for direct message match', async () => {
      const watcher = new MessageWatcher({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        onMessage: messageCallback
      });

      await watcher.start();

      // Create test message
      const message: Message = {
        id: 'test-msg-1',
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'Test message' },
        timestamp: new Date().toISOString()
      };

      // Write message file
      const filePath = path.join(testMessagesDir, `${message.id}.json`);
      fs.writeFileSync(filePath, JSON.stringify(message));

      // Wait for file watcher to trigger
      await new Promise(resolve => setTimeout(resolve, 200));

      expect(messageCallback).toHaveBeenCalledWith(message);

      watcher.stop();
    });

    it('should trigger callback for pattern match', async () => {
      const watcher = new MessageWatcher({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        onMessage: messageCallback
      });

      await watcher.start();

      // Create test message with pattern
      const message: Message = {
        id: 'test-msg-2',
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX-*',
        payload: { text: 'Pattern test' },
        timestamp: new Date().toISOString()
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${message.id}.json`),
        JSON.stringify(message)
      );

      await new Promise(resolve => setTimeout(resolve, 200));

      expect(messageCallback).toHaveBeenCalledWith(message);

      watcher.stop();
    });

    it('should trigger callback for broadcast message', async () => {
      const watcher = new MessageWatcher({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        onMessage: messageCallback
      });

      await watcher.start();

      // Create broadcast message
      const message: Message = {
        id: 'test-msg-3',
        from: { id: 'Agent1', name: 'Agent1' },
        to: '*',
        payload: { text: 'Broadcast test' },
        timestamp: new Date().toISOString()
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${message.id}.json`),
        JSON.stringify(message)
      );

      await new Promise(resolve => setTimeout(resolve, 200));

      expect(messageCallback).toHaveBeenCalledWith(message);

      watcher.stop();
    });

    it('should NOT trigger callback for non-matching message', async () => {
      const watcher = new MessageWatcher({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        onMessage: messageCallback
      });

      await watcher.start();

      // Create message for different agent
      const message: Message = {
        id: 'test-msg-4',
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'Agent2',
        payload: { text: 'Wrong agent test' },
        timestamp: new Date().toISOString()
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${message.id}.json`),
        JSON.stringify(message)
      );

      await new Promise(resolve => setTimeout(resolve, 200));

      expect(messageCallback).not.toHaveBeenCalled();

      watcher.stop();
    });
  });

  describe('duplicate prevention', () => {
    it('should not trigger callback twice for same message', async () => {
      const watcher = new MessageWatcher({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        onMessage: messageCallback
      });

      await watcher.start();

      const message: Message = {
        id: 'test-msg-5',
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'Duplicate test' },
        timestamp: new Date().toISOString()
      };

      const filePath = path.join(testMessagesDir, `${message.id}.json`);

      // Write file twice
      fs.writeFileSync(filePath, JSON.stringify(message));
      await new Promise(resolve => setTimeout(resolve, 200));

      // Try to trigger again (shouldn't work)
      fs.utimesSync(filePath, new Date(), new Date());
      await new Promise(resolve => setTimeout(resolve, 200));

      expect(messageCallback).toHaveBeenCalledTimes(1);

      watcher.stop();
    });
  });

  describe('lifecycle', () => {
    it('should create messages directory if not exists', async () => {
      const nonExistentDir = path.join(testMessagesDir, 'subdir');

      const watcher = new MessageWatcher({
        agentId: 'AgentX',
        messagesDir: nonExistentDir,
        onMessage: messageCallback
      });

      await watcher.start();

      expect(fs.existsSync(nonExistentDir)).toBe(true);

      watcher.stop();
    });

    it('should stop watching when stopped', async () => {
      const watcher = new MessageWatcher({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        onMessage: messageCallback
      });

      await watcher.start();
      watcher.stop();

      // Write message after stopping
      const message: Message = {
        id: 'test-msg-6',
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'After stop test' },
        timestamp: new Date().toISOString()
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${message.id}.json`),
        JSON.stringify(message)
      );

      await new Promise(resolve => setTimeout(resolve, 200));

      expect(messageCallback).not.toHaveBeenCalled();
    });
  });

  describe('priority handling', () => {
    it('should handle messages with priority field', async () => {
      const watcher = new MessageWatcher({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        onMessage: messageCallback
      });

      await watcher.start();

      const message: Message = {
        id: 'test-msg-7',
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'Urgent message' },
        timestamp: new Date().toISOString(),
        priority: 'urgent'
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${message.id}.json`),
        JSON.stringify(message)
      );

      await new Promise(resolve => setTimeout(resolve, 200));

      expect(messageCallback).toHaveBeenCalledWith(
        expect.objectContaining({ priority: 'urgent' })
      );

      watcher.stop();
    });
  });
});
