/**
 * Mock for node-pty module
 * Used in tests to avoid native module dependency
 */

export const spawn = jest.fn(() => ({
  pid: 12345,
  onData: jest.fn(),
  write: jest.fn(),
  resize: jest.fn(),
  kill: jest.fn()
}));
