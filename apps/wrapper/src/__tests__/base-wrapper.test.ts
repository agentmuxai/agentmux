import { BaseWrapper } from '../wrappers/base';
import { Message, WrapperOptions } from '../types';

// Mock node-pty
jest.mock('node-pty', () => ({
  spawn: jest.fn(() => ({
    pid: 12345,
    onData: jest.fn(),
    write: jest.fn(),
    resize: jest.fn(),
    kill: jest.fn()
  }))
}));

// Mock message watcher
jest.mock('../watcher', () => ({
  MessageWatcher: jest.fn().mockImplementation(() => ({
    start: jest.fn().mockResolvedValue(undefined),
    stop: jest.fn()
  }))
}));

// Test implementation of BaseWrapper
class TestWrapper extends BaseWrapper {
  get command(): string {
    return 'test-cli';
  }
}

describe('BaseWrapper', () => {
  let originalStdin: any;
  let originalStdout: any;

  beforeEach(() => {
    // Mock stdin/stdout
    originalStdin = process.stdin;
    originalStdout = process.stdout;

    process.stdin = {
      setRawMode: jest.fn(),
      on: jest.fn()
    } as any;

    process.stdout = {
      write: jest.fn(),
      on: jest.fn(),
      columns: 80,
      rows: 30
    } as any;
  });

  afterEach(() => {
    // Restore stdin/stdout
    process.stdin = originalStdin;
    process.stdout = originalStdout;
  });

  describe('initialization', () => {
    it('should initialize with default options', () => {
      const wrapper = new TestWrapper();
      expect(wrapper).toBeDefined();
    });

    it('should use provided agent ID', () => {
      const options: WrapperOptions = {
        agentId: 'Agent5',
        debug: false
      };

      const wrapper = new TestWrapper(options);
      expect(wrapper).toBeDefined();
    });

    it('should use AGENT_ID env var if no agentId provided', () => {
      process.env.AGENT_ID = 'AgentFromEnv';
      const wrapper = new TestWrapper();
      expect(wrapper).toBeDefined();
      delete process.env.AGENT_ID;
    });
  });

  describe('message handling', () => {
    it('should show notification for normal priority message', async () => {
      const wrapper = new TestWrapper({ debug: false });
      await wrapper.start();

      const mockPty = (wrapper as any).ptyProcess;
      mockPty.write = jest.fn();

      const message: Message = {
        id: 'test-msg-1',
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'Test' },
        timestamp: new Date().toISOString()
      };

      // Trigger handleMessage
      (wrapper as any).handleMessage(message);

      expect(mockPty.write).toHaveBeenCalledWith('\n');
      expect(mockPty.write).toHaveBeenCalledWith(
        expect.stringContaining('Remote message from Agent1')
      );
      expect(mockPty.write).toHaveBeenCalledWith('check messages\n');

      wrapper.stop();
    });

    it('should show urgent notification for urgent priority message', async () => {
      const wrapper = new TestWrapper({ debug: false });
      await wrapper.start();

      const mockPty = (wrapper as any).ptyProcess;
      mockPty.write = jest.fn();

      const message: Message = {
        id: 'test-msg-2',
        from: { id: 'Agent1', name: 'Agent1' },
        to: 'AgentX',
        payload: { text: 'Urgent!' },
        timestamp: new Date().toISOString(),
        priority: 'urgent'
      };

      (wrapper as any).handleMessage(message);

      expect(mockPty.write).toHaveBeenCalledWith(
        expect.stringContaining('⚠️')
      );

      wrapper.stop();
    });
  });

  describe('command injection', () => {
    it('should inject command to PTY process', async () => {
      const wrapper = new TestWrapper({ debug: false });
      await wrapper.start();

      const mockPty = (wrapper as any).ptyProcess;
      mockPty.write = jest.fn();

      wrapper.inject('test command');

      expect(mockPty.write).toHaveBeenCalledWith('test command\n');

      wrapper.stop();
    });

    it('should throw error if PTY not initialized', () => {
      const wrapper = new TestWrapper({ debug: false });

      expect(() => wrapper.inject('test')).toThrow('PTY process not initialized');
    });
  });

  describe('lifecycle', () => {
    it('should start successfully', async () => {
      const wrapper = new TestWrapper({ debug: false });
      await expect(wrapper.start()).resolves.not.toThrow();
      wrapper.stop();
    });

    it('should stop successfully', async () => {
      const wrapper = new TestWrapper({ debug: false });
      await wrapper.start();

      expect(() => wrapper.stop()).not.toThrow();
    });

    it('should restore terminal on stop', async () => {
      const setRawMode = jest.fn();
      process.stdin.setRawMode = setRawMode;

      const wrapper = new TestWrapper({ debug: false });
      await wrapper.start();
      wrapper.stop();

      expect(setRawMode).toHaveBeenCalledWith(false);
    });
  });

  describe('debug logging', () => {
    it('should not log when debug is false', () => {
      const consoleError = jest.spyOn(console, 'error').mockImplementation();

      const wrapper = new TestWrapper({ debug: false });
      (wrapper as any).log('Test message', { data: 'value' });

      expect(consoleError).not.toHaveBeenCalled();

      consoleError.mockRestore();
    });

    it('should log when debug is true', () => {
      const consoleError = jest.spyOn(console, 'error').mockImplementation();

      const wrapper = new TestWrapper({ debug: true });
      (wrapper as any).log('Test message', { data: 'value' });

      expect(consoleError).toHaveBeenCalledWith(
        expect.stringContaining('[BaseWrapper] Test message'),
        expect.stringContaining('"data":"value"')
      );

      consoleError.mockRestore();
    });
  });
});
