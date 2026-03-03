import { waitForAppReady, byTestId, waitForElement } from '../helpers/tauri-helpers.js'

describe('Layout', () => {
  before(async () => {
    await waitForAppReady()
  })

  it('should have at least one block pane', async () => {
    await browser.waitUntil(
      async () => {
        const blocks = await $$('[data-blockid]')
        return blocks.length > 0
      },
      {
        timeout: 10000,
        timeoutMsg: 'No block panes found'
      }
    )

    const blocks = await $$('[data-blockid]')
    expect(blocks.length).toBeGreaterThan(0)
  })

  it('should have visible block headers', async () => {
    const selector = byTestId('block-header')
    await waitForElement(selector)

    const headers = await $$(selector)
    for (const header of headers) {
      expect(await header.isDisplayed()).toBe(true)
    }
  })
})
