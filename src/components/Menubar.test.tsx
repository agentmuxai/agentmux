import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@solidjs/testing-library';
import Menubar from './Menubar';

describe('Menubar', () => {
  it('renders hamburger menu button', () => {
    const mockFn = vi.fn();
    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={mockFn}
        onShowAgentList={mockFn}
        onShowMessageStream={mockFn}
      />
    ));

    const toggle = screen.getByTestId('menubar-toggle');
    expect(toggle).toBeTruthy();
  });

  it('opens dropdown when toggle clicked', () => {
    const mockFn = vi.fn();
    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={mockFn}
        onShowAgentList={mockFn}
        onShowMessageStream={mockFn}
      />
    ));

    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    const dropdown = screen.getByTestId('menubar-dropdown');
    expect(dropdown).toBeTruthy();
  });

  it('closes dropdown when overlay clicked', () => {
    const mockFn = vi.fn();
    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={mockFn}
        onShowAgentList={mockFn}
        onShowMessageStream={mockFn}
      />
    ));

    // Open menu
    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    expect(screen.getByTestId('menubar-dropdown')).toBeTruthy();

    // Click overlay
    const overlay = document.querySelector('.menubar-overlay');
    fireEvent.click(overlay!);

    // Dropdown should be gone
    expect(screen.queryByTestId('menubar-dropdown')).toBeFalsy();
  });

  it('calls onShowDashboard when Dashboard clicked', () => {
    const onShowDashboard = vi.fn();
    const mockFn = vi.fn();

    render(() => (
      <Menubar
        onShowDashboard={onShowDashboard}
        onShowBusInfo={mockFn}
        onShowAgentList={mockFn}
        onShowMessageStream={mockFn}
      />
    ));

    // Open menu
    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    // Click Dashboard
    const dashboard = screen.getByTestId('menu-dashboard');
    fireEvent.click(dashboard);

    expect(onShowDashboard).toHaveBeenCalled();
  });

  it('calls onShowBusInfo when Bus Info clicked', () => {
    const onShowBusInfo = vi.fn();
    const mockFn = vi.fn();

    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={onShowBusInfo}
        onShowAgentList={mockFn}
        onShowMessageStream={mockFn}
      />
    ));

    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    const busInfo = screen.getByTestId('menu-bus');
    fireEvent.click(busInfo);

    expect(onShowBusInfo).toHaveBeenCalled();
  });

  it('calls onShowAgentList when Agent List clicked', () => {
    const onShowAgentList = vi.fn();
    const mockFn = vi.fn();

    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={mockFn}
        onShowAgentList={onShowAgentList}
        onShowMessageStream={mockFn}
      />
    ));

    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    const agentList = screen.getByTestId('menu-agents');
    fireEvent.click(agentList);

    expect(onShowAgentList).toHaveBeenCalled();
  });

  it('calls onShowMessageStream when Message Stream clicked', () => {
    const onShowMessageStream = vi.fn();
    const mockFn = vi.fn();

    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={mockFn}
        onShowAgentList={mockFn}
        onShowMessageStream={onShowMessageStream}
      />
    ));

    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    const messageStream = screen.getByTestId('menu-messages');
    fireEvent.click(messageStream);

    expect(onShowMessageStream).toHaveBeenCalled();
  });

  it('closes menu after action', () => {
    const mockFn = vi.fn();
    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={mockFn}
        onShowAgentList={mockFn}
        onShowMessageStream={mockFn}
      />
    ));

    // Open menu
    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    expect(screen.getByTestId('menubar-dropdown')).toBeTruthy();

    // Click an action
    const dashboard = screen.getByTestId('menu-dashboard');
    fireEvent.click(dashboard);

    // Menu should close
    expect(screen.queryByTestId('menubar-dropdown')).toBeFalsy();
  });

  describe('Layout Actions', () => {
    beforeEach(() => {
      // Setup window.paneActions mock
      (window as any).paneActions = {
        splitVertical: vi.fn(),
        splitHorizontal: vi.fn(),
        closeCurrentPane: vi.fn(),
        resetToSingle: vi.fn(),
      };
    });

    afterEach(() => {
      delete (window as any).paneActions;
    });

    it('calls splitVertical when Split Vertical clicked', () => {
      const mockFn = vi.fn();
      render(() => (
        <Menubar
          onShowDashboard={mockFn}
          onShowBusInfo={mockFn}
          onShowAgentList={mockFn}
          onShowMessageStream={mockFn}
        />
      ));

      const toggle = screen.getByTestId('menubar-toggle');
      fireEvent.click(toggle);

      const splitVertical = screen.getByTestId('menu-split-vertical');
      fireEvent.click(splitVertical);

      expect((window as any).paneActions.splitVertical).toHaveBeenCalled();
    });

    it('calls splitHorizontal when Split Horizontal clicked', () => {
      const mockFn = vi.fn();
      render(() => (
        <Menubar
          onShowDashboard={mockFn}
          onShowBusInfo={mockFn}
          onShowAgentList={mockFn}
          onShowMessageStream={mockFn}
        />
      ));

      const toggle = screen.getByTestId('menubar-toggle');
      fireEvent.click(toggle);

      const splitHorizontal = screen.getByTestId('menu-split-horizontal');
      fireEvent.click(splitHorizontal);

      expect((window as any).paneActions.splitHorizontal).toHaveBeenCalled();
    });

    it('disables Close Current Pane when paneCount <= 1', () => {
      const mockFn = vi.fn();
      render(() => (
        <Menubar
          onShowDashboard={mockFn}
          onShowBusInfo={mockFn}
          onShowAgentList={mockFn}
          onShowMessageStream={mockFn}
          paneCount={1}
        />
      ));

      const toggle = screen.getByTestId('menubar-toggle');
      fireEvent.click(toggle);

      const closePane = screen.getByTestId('menu-close-pane') as HTMLButtonElement;
      expect(closePane.disabled).toBe(true);
    });

    it('enables Close Current Pane when paneCount > 1', () => {
      const mockFn = vi.fn();
      render(() => (
        <Menubar
          onShowDashboard={mockFn}
          onShowBusInfo={mockFn}
          onShowAgentList={mockFn}
          onShowMessageStream={mockFn}
          paneCount={2}
        />
      ));

      const toggle = screen.getByTestId('menubar-toggle');
      fireEvent.click(toggle);

      const closePane = screen.getByTestId('menu-close-pane') as HTMLButtonElement;
      expect(closePane.disabled).toBe(false);
    });

    it('disables Reset to Single Pane when paneCount <= 1', () => {
      const mockFn = vi.fn();
      render(() => (
        <Menubar
          onShowDashboard={mockFn}
          onShowBusInfo={mockFn}
          onShowAgentList={mockFn}
          onShowMessageStream={mockFn}
          paneCount={1}
        />
      ));

      const toggle = screen.getByTestId('menubar-toggle');
      fireEvent.click(toggle);

      const reset = screen.getByTestId('menu-reset-layout') as HTMLButtonElement;
      expect(reset.disabled).toBe(true);
    });

    it('enables Reset to Single Pane when paneCount > 1', () => {
      const mockFn = vi.fn();
      render(() => (
        <Menubar
          onShowDashboard={mockFn}
          onShowBusInfo={mockFn}
          onShowAgentList={mockFn}
          onShowMessageStream={mockFn}
          paneCount={3}
        />
      ));

      const toggle = screen.getByTestId('menubar-toggle');
      fireEvent.click(toggle);

      const reset = screen.getByTestId('menu-reset-layout') as HTMLButtonElement;
      expect(reset.disabled).toBe(false);
    });
  });

  it('opens documentation link in new tab', () => {
    const mockFn = vi.fn();
    const windowOpen = vi.spyOn(window, 'open').mockImplementation(() => null);

    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={mockFn}
        onShowAgentList={mockFn}
        onShowMessageStream={mockFn}
      />
    ));

    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    const docs = screen.getByTestId('menu-docs');
    fireEvent.click(docs);

    expect(windowOpen).toHaveBeenCalledWith('https://github.com/a5af/agentmux', '_blank');

    windowOpen.mockRestore();
  });

  it('shows about alert when About clicked', () => {
    const mockFn = vi.fn();
    const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {});

    render(() => (
      <Menubar
        onShowDashboard={mockFn}
        onShowBusInfo={mockFn}
        onShowAgentList={mockFn}
        onShowMessageStream={mockFn}
      />
    ));

    const toggle = screen.getByTestId('menubar-toggle');
    fireEvent.click(toggle);

    const about = screen.getByTestId('menu-about');
    fireEvent.click(about);

    expect(alertSpy).toHaveBeenCalledWith(expect.stringContaining('AgentMux Desktop'));

    alertSpy.mockRestore();
  });
});
