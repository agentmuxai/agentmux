import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { MessageWatcher } from '../watcher';
import { BaseWrapper } from '../wrappers/base';
import { Message } from '../types';

// Mock PTY for integration tests
jest.mock('node-pty', () => ({
  spawn: jest.fn(() => ({
    pid: 12345,
    onData: jest.fn(),
    write: jest.fn(),
    resize: jest.fn(),
    kill: jest.fn()
  }))
}));

class TestWrapper extends BaseWrapper {
  get command(): string {
    return 'test-cli';
  }

  public getInjectedCommands(): string[] {
    const mockPty = (this as any).ptyProcess;
    return mockPty.write.mock.calls
      .filter((call: any[]) => call[0].endsWith('\n'))
      .map((call: any[]) => call[0].trim());
  }
}

describe('Integration: Message Flow', () => {
  let testMessagesDir: string;

  beforeEach(() => {
    testMessagesDir = path.join(os.tmpdir(), `agentmux-integration-${Date.now()}`);
    fs.mkdirSync(testMessagesDir, { recursive: true });

    // Mock stdin/stdout methods
    jest.spyOn(process.stdin, 'setRawMode').mockImplementation(() => process.stdin as any);
    jest.spyOn(process.stdin, 'on').mockImplementation(() => process.stdin);
    jest.spyOn(process.stdout, 'write').mockImplementation(() => true as any);
    jest.spyOn(process.stdout, 'on').mockImplementation(() => process.stdout);
  });

  afterEach(() => {
    jest.restoreAllMocks();

    if (fs.existsSync(testMessagesDir)) {
      fs.rmSync(testMessagesDir, { recursive: true, force: true });
    }
  });

  describe('end-to-end message notification', () => {
    it('should detect message and inject check command', async () => {
      const wrapper = new TestWrapper({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        debug: false
      });

      await wrapper.start();

      // Simulate Agent1 sending message
      const message: Message = {
        id: `msg-${Date.now()}`,
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'Review PR #156' },
        timestamp: new Date().toISOString()
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${message.id}.json`),
        JSON.stringify(message)
      );

      // Wait for file watcher to trigger
      await new Promise(resolve => setTimeout(resolve, 600));

      // Verify command was injected
      const injectedCommands = wrapper.getInjectedCommands();
      expect(injectedCommands).toContain('check messages');

      wrapper.stop();
      await new Promise(resolve => setTimeout(resolve, 100));
    }, 10000);

    it('should handle multiple messages in sequence', async () => {
      const wrapper = new TestWrapper({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        debug: false
      });

      await wrapper.start();

      // Send multiple messages
      const messages: Message[] = [
        {
          id: `msg-1-${Date.now()}`,
          from: { id: 'Agent1', name: 'Agent1' },
          to: 'AgentX',
          payload: { text: 'Message 1' },
          timestamp: new Date().toISOString()
        },
        {
          id: `msg-2-${Date.now()}`,
          from: { id: 'Agent2', name: 'Agent2' },
          to: 'AgentX',
          payload: { text: 'Message 2' },
          timestamp: new Date().toISOString()
        },
        {
          id: `msg-3-${Date.now()}`,
          from: { id: 'Agent3', name: 'Agent3' },
          to: 'AgentX',
          payload: { text: 'Message 3' },
          timestamp: new Date().toISOString()
        }
      ];

      for (const message of messages) {
        fs.writeFileSync(
          path.join(testMessagesDir, `${message.id}.json`),
          JSON.stringify(message)
        );
        await new Promise(resolve => setTimeout(resolve, 300));
      }

      // Wait for all to process
      await new Promise(resolve => setTimeout(resolve, 1000));

      // Verify all were processed
      const injectedCommands = wrapper.getInjectedCommands();
      const checkMessageCommands = injectedCommands.filter(cmd => cmd === 'check messages');
      expect(checkMessageCommands.length).toBe(3);

      wrapper.stop();
      await new Promise(resolve => setTimeout(resolve, 100));
    }, 15000);

    it('should filter messages not addressed to this agent', async () => {
      const wrapper = new TestWrapper({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        debug: false
      });

      await wrapper.start();

      // Message for different agent
      const wrongMessage: Message = {
        id: `msg-wrong-${Date.now()}`,
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'Agent2',
        payload: { text: 'For Agent2 only' },
        timestamp: new Date().toISOString()
      };

      // Message for this agent
      const rightMessage: Message = {
        id: `msg-right-${Date.now()}`,
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'For AgentX' },
        timestamp: new Date().toISOString()
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${wrongMessage.id}.json`),
        JSON.stringify(wrongMessage)
      );
      await new Promise(resolve => setTimeout(resolve, 600));

      fs.writeFileSync(
        path.join(testMessagesDir, `${rightMessage.id}.json`),
        JSON.stringify(rightMessage)
      );
      await new Promise(resolve => setTimeout(resolve, 600));

      // Should only inject command once (for right message)
      const injectedCommands = wrapper.getInjectedCommands();
      const checkMessageCommands = injectedCommands.filter(cmd => cmd === 'check messages');
      expect(checkMessageCommands.length).toBe(1);

      wrapper.stop();
      await new Promise(resolve => setTimeout(resolve, 100));
    }, 10000);
  });

  describe('pattern matching', () => {
    it('should handle wildcard patterns', async () => {
      const wrapper = new TestWrapper({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        debug: false
      });

      await wrapper.start();

      const message: Message = {
        id: `msg-pattern-${Date.now()}`,
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX-*',
        payload: { text: 'Pattern test' },
        timestamp: new Date().toISOString()
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${message.id}.json`),
        JSON.stringify(message)
      );

      await new Promise(resolve => setTimeout(resolve, 300));

      const injectedCommands = wrapper.getInjectedCommands();
      expect(injectedCommands).toContain('check messages');

      wrapper.stop();
      await new Promise(resolve => setTimeout(resolve, 100));
    }, 10000);

    it('should handle broadcast messages', async () => {
      const wrapper = new TestWrapper({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        debug: false
      });

      await wrapper.start();

      const message: Message = {
        id: `msg-broadcast-${Date.now()}`,
        from: { id: 'Agent1', name: 'Agent1' },
        to: '*',
        payload: { text: 'Broadcast to all' },
        timestamp: new Date().toISOString()
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${message.id}.json`),
        JSON.stringify(message)
      );

      await new Promise(resolve => setTimeout(resolve, 300));

      const injectedCommands = wrapper.getInjectedCommands();
      expect(injectedCommands).toContain('check messages');

      wrapper.stop();
      await new Promise(resolve => setTimeout(resolve, 100));
    }, 10000);
  });

  describe('priority notifications', () => {
    it('should handle urgent and normal priority messages', async () => {
      const wrapper = new TestWrapper({
        agentId: 'AgentX',
        messagesDir: testMessagesDir,
        debug: false
      });

      await wrapper.start();

      const mockPty = (wrapper as any).ptyProcess;
      mockPty.write = jest.fn();

      // Normal message
      const normalMsg: Message = {
        id: `msg-normal-${Date.now()}`,
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'Normal message' },
        timestamp: new Date().toISOString(),
        priority: 'normal'
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${normalMsg.id}.json`),
        JSON.stringify(normalMsg)
      );

      await new Promise(resolve => setTimeout(resolve, 300));

      // Check for blue background
      expect(mockPty.write).toHaveBeenCalledWith(
        expect.stringContaining('\x1b[44m')
      );

      mockPty.write.mockClear();

      // Urgent message
      const urgentMsg: Message = {
        id: `msg-urgent-${Date.now()}`,
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'Urgent message' },
        timestamp: new Date().toISOString(),
        priority: 'urgent'
      };

      fs.writeFileSync(
        path.join(testMessagesDir, `${urgentMsg.id}.json`),
        JSON.stringify(urgentMsg)
      );

      await new Promise(resolve => setTimeout(resolve, 300));

      // Check for red background
      expect(mockPty.write).toHaveBeenCalledWith(
        expect.stringContaining('\x1b[41m')
      );

      wrapper.stop();
      await new Promise(resolve => setTimeout(resolve, 100));
    }, 10000);
  });
});
