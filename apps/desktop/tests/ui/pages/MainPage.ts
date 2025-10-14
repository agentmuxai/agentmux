import { Page, Locator } from '@playwright/test';
import { BasePage } from './BasePage';

/**
 * MainPage - Page Object for the main AgentMux desktop application window
 *
 * Provides methods to interact with the main application UI elements.
 */
export class MainPage extends BasePage {
  // Locators for main UI elements
  readonly agentsList: Locator;
  readonly messagesList: Locator;
  readonly debugConsole: Locator;
  readonly statusBar: Locator;

  constructor(page: Page) {
    super(page);

    // Define locators (adjust selectors based on actual UI structure)
    this.agentsList = page.locator('[data-testid="agents-list"]');
    this.messagesList = page.locator('[data-testid="messages-list"]');
    this.debugConsole = page.locator('[data-testid="debug-console"]');
    this.statusBar = page.locator('[data-testid="status-bar"]');
  }

  /**
   * Navigate to the main application
   */
  async open(): Promise<void> {
    await this.goto('/');
    await this.waitForLoad();
  }

  /**
   * Wait for the application to be fully initialized
   */
  async waitForAppReady(): Promise<void> {
    await this.waitForSelector('[data-testid="app-ready"]', 10000);
  }

  /**
   * Get the list of visible agents
   */
  async getAgents(): Promise<string[]> {
    const agentElements = await this.agentsList.locator('[data-testid="agent-item"]').all();
    return Promise.all(agentElements.map(el => this.getTextContent(el)));
  }

  /**
   * Get the list of visible messages
   */
  async getMessages(): Promise<string[]> {
    const messageElements = await this.messagesList.locator('[data-testid="message-item"]').all();
    return Promise.all(messageElements.map(el => this.getTextContent(el)));
  }

  /**
   * Check if debug console is visible
   */
  async isDebugConsoleVisible(): Promise<boolean> {
    return await this.isVisible(this.debugConsole);
  }

  /**
   * Toggle debug console visibility
   */
  async toggleDebugConsole(): Promise<void> {
    const toggleButton = this.page.locator('[data-testid="debug-console-toggle"]');
    await this.clickWithRetry(toggleButton);
  }

  /**
   * Get debug console output
   */
  async getDebugOutput(): Promise<string> {
    return await this.getTextContent(this.debugConsole);
  }

  /**
   * Get status bar text
   */
  async getStatusText(): Promise<string> {
    return await this.getTextContent(this.statusBar);
  }

  /**
   * Wait for a specific number of agents to be visible
   */
  async waitForAgentCount(count: number, timeout = 5000): Promise<void> {
    await this.page.waitForFunction(
      (expectedCount) => {
        const agents = document.querySelectorAll('[data-testid="agent-item"]');
        return agents.length === expectedCount;
      },
      count,
      { timeout }
    );
  }

  /**
   * Wait for a specific number of messages to be visible
   */
  async waitForMessageCount(count: number, timeout = 5000): Promise<void> {
    await this.page.waitForFunction(
      (expectedCount) => {
        const messages = document.querySelectorAll('[data-testid="message-item"]');
        return messages.length === expectedCount;
      },
      count,
      { timeout }
    );
  }

  /**
   * Send a message via the message input (if applicable)
   */
  async sendMessage(message: string): Promise<void> {
    const messageInput = this.page.locator('[data-testid="message-input"]');
    await this.typeText(messageInput, message);

    const sendButton = this.page.locator('[data-testid="send-button"]');
    await this.clickWithRetry(sendButton);
  }
}
