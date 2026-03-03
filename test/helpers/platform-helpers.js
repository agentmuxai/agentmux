/**
 * Cross-platform helper functions for Tauri testing
 * Use these in all tests for platform detection and platform-aware behavior
 */

/**
 * Platform detection utilities
 */
export function isWindows() {
  return process.platform === 'win32'
}

export function isLinux() {
  return process.platform === 'linux'
}

export function isMac() {
  return process.platform === 'darwin'
}

/**
 * Get platform name as string
 */
export function getPlatformName() {
  if (isWindows()) return 'Windows'
  if (isLinux()) return 'Linux'
  if (isMac()) return 'macOS'
  return process.platform
}

/**
 * Get platform-aware wait time
 * Windows often needs longer waits for animations/rendering
 *
 * @param {number} baseMs - Base wait time in milliseconds
 * @param {number} windowsMultiplier - Multiplier for Windows (default 1.5)
 * @returns {number} Platform-adjusted wait time in milliseconds
 */
export function getWaitTime(baseMs, windowsMultiplier = 1.5) {
  return isWindows() ? Math.floor(baseMs * windowsMultiplier) : baseMs
}

/**
 * Get platform-aware modifier key
 * Returns 'Control' on Windows/Linux, 'Command' on macOS
 *
 * @returns {string} Modifier key name for WebdriverIO
 */
export function getModifierKey() {
  return isMac() ? 'Command' : 'Control'
}

/**
 * Get platform-aware application binary path
 *
 * @param {string} basePath - Path without extension (e.g., './target/release/agentmux')
 * @returns {string} Platform-specific path
 */
export function getAppPath(basePath) {
  return isWindows() ? `${basePath}.exe` : basePath
}

/**
 * Conditional test skip based on platform
 *
 * @param {string} platform - Platform to run on: 'windows', 'linux', 'mac'
 * @param {object} testContext - Mocha test context (this)
 * @returns {boolean} Whether test was skipped
 */
export function skipUnless(platform, testContext) {
  const shouldSkip = (
    (platform === 'windows' && !isWindows()) ||
    (platform === 'linux' && !isLinux()) ||
    (platform === 'mac' && !isMac())
  )

  if (shouldSkip && testContext) {
    testContext.skip()
  }

  return shouldSkip
}

/**
 * Conditional test skip for multiple platforms
 *
 * @param {string[]} platforms - Platforms to run on: ['windows', 'linux', 'mac']
 * @param {object} testContext - Mocha test context (this)
 * @returns {boolean} Whether test was skipped
 */
export function skipUnlessAny(platforms, testContext) {
  const shouldRun = platforms.some(platform => {
    if (platform === 'windows') return isWindows()
    if (platform === 'linux') return isLinux()
    if (platform === 'mac') return isMac()
    return false
  })

  if (!shouldRun && testContext) {
    testContext.skip()
  }

  return !shouldRun
}

/**
 * Get platform from browser environment
 * Useful for runtime platform detection
 *
 * @returns {Promise<string>} Platform string from navigator.platform
 */
export async function getPlatformFromBrowser() {
  return await browser.execute(() => navigator.platform)
}

/**
 * Log platform information for debugging
 */
export function logPlatformInfo() {
  console.log(`Platform: ${getPlatformName()} (${process.platform})`)
  console.log(`Node version: ${process.version}`)
  console.log(`Architecture: ${process.arch}`)
}
