import { jest } from '@jest/globals';
import { spawn } from 'child_process';
import fs from 'fs';
import path from 'path';
import os from 'os';

// Mock child_process and fs
jest.mock('child_process');
jest.mock('fs');

describe('ReactiveCLIAgent', () => {
  let mockProcess;
  let mockStdout;
  let mockStderr;
  let mockStdin;

  beforeEach(() => {
    // Setup mock process
    mockStdout = {
      on: jest.fn(),
    };
    mockStderr = {
      on: jest.fn(),
    };
    mockStdin = {
      write: jest.fn(),
    };
    mockProcess = {
      stdout: mockStdout,
      stderr: mockStderr,
      stdin: mockStdin,
      pid: 12345,
      on: jest.fn(),
    };

    spawn.mockReturnValue(mockProcess);

    // Setup fs mocks
    fs.existsSync.mockReturnValue(true);
    fs.mkdirSync.mockImplementation(() => {});
    fs.readdirSync.mockReturnValue([]);
    fs.watch.mockReturnValue({ close: jest.fn() });
    fs.writeFileSync.mockImplementation(() => {});
    fs.appendFileSync.mockImplementation(() => {});
  });

  afterEach(() => {
    jest.clearAllMocks();
  });

  describe('Output Capture', () => {
    test('captures stdout and stores in buffer', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();

      // Simulate output
      const outputCallback = mockStdout.on.mock.calls.find(
        call => call[0] === 'data'
      )[1];

      outputCallback(Buffer.from('Hello from Claude\n'));

      expect(agent.fullOutput).toContain('Hello from Claude');
    });

    test('writes output to live output file', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();

      const outputCallback = mockStdout.on.mock.calls.find(
        call => call[0] === 'data'
      )[1];

      outputCallback(Buffer.from('Test output\n'));

      expect(fs.writeFileSync).toHaveBeenCalledWith(
        expect.stringContaining('live-output.txt'),
        expect.stringContaining('Test output')
      );
    });

    test('appends to log file with timestamp', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();

      const outputCallback = mockStdout.on.mock.calls.find(
        call => call[0] === 'data'
      )[1];

      outputCallback(Buffer.from('Logged output\n'));

      expect(fs.appendFileSync).toHaveBeenCalledWith(
        expect.stringContaining('agent.log'),
        expect.stringMatching(/\[\d{4}-\d{2}-\d{2}T.*\] Logged output/)
      );
    });
  });

  describe('Message Processing', () => {
    test('detects messages addressed to agent', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('Agent1', 'echo');

      const message = {
        to: 'Agent1',
        from: { id: 'Desktop' },
        payload: { text: 'Hello' },
      };

      expect(agent.isMessageForMe(message)).toBe(true);
    });

    test('detects broadcast messages', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('Agent1', 'echo');

      const message = {
        to: '*',
        from: { id: 'Desktop' },
        payload: { text: 'Hello all' },
      };

      expect(agent.isMessageForMe(message)).toBe(true);
    });

    test('detects wildcard pattern messages', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('Agent123', 'echo');

      const message = {
        to: 'Agent*',
        from: { id: 'Desktop' },
        payload: { text: 'Hello agents' },
      };

      expect(agent.isMessageForMe(message)).toBe(true);
    });

    test('ignores messages for other agents', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('Agent1', 'echo');

      const message = {
        to: 'Agent2',
        from: { id: 'Desktop' },
        payload: { text: 'Hello Agent2' },
      };

      expect(agent.isMessageForMe(message)).toBe(false);
    });

    test('ignores own messages', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('Agent1', 'echo');

      const message = {
        to: 'Desktop',
        from: { id: 'Agent1' },
        payload: { text: 'Response' },
      };

      expect(agent.isMessageForMe(message)).toBe(false);
    });

    test('injects message into stdin', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('Agent1', 'echo');

      await agent.start();
      agent.process = mockProcess;

      const message = {
        to: 'Agent1',
        from: { id: 'Desktop' },
        payload: { text: 'What is 2+2?' },
      };

      agent.messageQueue.push(message);
      await agent.processNextMessage();

      expect(mockStdin.write).toHaveBeenCalledWith('What is 2+2?\n');
    });
  });

  describe('Status Tracking', () => {
    test('creates status file on start', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();

      expect(fs.writeFileSync).toHaveBeenCalledWith(
        expect.stringContaining('status.json'),
        expect.stringContaining('"agentId":"TestAgent"')
      );
    });

    test('updates status with message count', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();
      agent.processedMessages.add('msg1.json');
      agent.processedMessages.add('msg2.json');
      agent.updateStatus('running');

      const statusCall = fs.writeFileSync.mock.calls.find(
        call => call[0].includes('status.json')
      );

      expect(statusCall[1]).toContain('"messagesReceived":2');
    });

    test('tracks output length', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();
      agent.fullOutput = 'Some output text';
      agent.updateStatus('running');

      const statusCall = fs.writeFileSync.mock.calls.find(
        call => call[0].includes('status.json')
      );

      expect(statusCall[1]).toContain('"outputLength":16');
    });
  });

  describe('Error Handling', () => {
    test('captures stderr output', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();

      const errorCallback = mockStderr.on.mock.calls.find(
        call => call[0] === 'data'
      )[1];

      errorCallback(Buffer.from('Error message\n'));

      expect(fs.appendFileSync).toHaveBeenCalledWith(
        expect.stringContaining('agent.log'),
        expect.stringContaining('ERROR: Error message')
      );
    });

    test('handles process exit', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();

      const exitCallback = mockProcess.on.mock.calls.find(
        call => call[0] === 'exit'
      )[1];

      const mockExit = jest.spyOn(process, 'exit').mockImplementation(() => {});

      exitCallback(0);

      // Verify exit was called
      expect(mockExit).toHaveBeenCalledWith(0);

      mockExit.mockRestore();
    });
  });

  describe('Directory Management', () => {
    test('creates required directories', async () => {
      fs.existsSync.mockReturnValue(false);

      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();

      expect(fs.mkdirSync).toHaveBeenCalledWith(
        expect.stringContaining('.agentmux'),
        { recursive: true }
      );
    });

    test('skips directory creation if exists', async () => {
      fs.existsSync.mockReturnValue(true);

      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('TestAgent', 'echo');

      await agent.start();

      expect(fs.mkdirSync).not.toHaveBeenCalled();
    });
  });

  describe('Message Sending', () => {
    test('creates message file with correct structure', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('Agent1', 'echo');

      agent.sendMessage('Desktop', 'Hello Desktop');

      const messageCall = fs.writeFileSync.mock.calls.find(
        call => call[0].includes('.json') && call[1].includes('Hello Desktop')
      );

      expect(messageCall).toBeDefined();

      const messageData = JSON.parse(messageCall[1]);
      expect(messageData).toMatchObject({
        from: {
          id: 'Agent1',
          name: 'Agent1 (Claude)',
        },
        to: 'Desktop',
        payload: {
          text: 'Hello Desktop',
        },
        priority: 'normal',
      });
    });

    test('marks own messages as processed', async () => {
      const { ReactiveCLIAgent } = await import('../reactive-claude-agent.js');
      const agent = new ReactiveCLIAgent('Agent1', 'echo');

      agent.sendMessage('Desktop', 'Test');

      const messageCall = fs.writeFileSync.mock.calls.find(
        call => call[0].includes('.json')
      );

      const filename = path.basename(messageCall[0]);
      expect(agent.processedMessages.has(filename)).toBe(true);
    });
  });
});
