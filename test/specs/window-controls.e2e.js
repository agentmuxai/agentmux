import { waitForAppReady, byTestId, waitForElement } from '../helpers/tauri-helpers.js'

describe('Window Controls', () => {
  before(async () => {
    await waitForAppReady()
  })

  it('should show the agentmux button', async () => {
    const selector = byTestId('new-window-btn')
    await waitForElement(selector)

    const btn = await $(selector)
    expect(await btn.isDisplayed()).toBe(true)
    expect(await btn.getText()).toContain('agentmux')
  })

  it('should show the window header', async () => {
    const selector = byTestId('window-header')
    await waitForElement(selector)

    const header = await $(selector)
    expect(await header.isDisplayed()).toBe(true)
  })

  it('should show action widgets', async () => {
    const selector = byTestId('action-widgets')
    await waitForElement(selector)

    const widgets = await $(selector)
    expect(await widgets.isDisplayed()).toBe(true)
  })

  it('should show window controls container', async () => {
    const selector = byTestId('window-controls')
    await waitForElement(selector)

    const controls = await $(selector)
    expect(await controls.isDisplayed()).toBe(true)
  })
})
