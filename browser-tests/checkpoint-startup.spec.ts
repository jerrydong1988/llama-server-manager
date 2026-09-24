import { expect, test } from '@playwright/test'

const peerError = 'runtime pipe server is not this application'
const hydrationWarning = `checkpoint status hydration failed: ${peerError}`

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('lang', 'zh-CN'))
})

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('only a complete checkpoint snapshot clears a failed startup hydration', async ({ page }) => {
  await page.goto('/?scenario=checkpoint-hydration-delayed')
  await page.evaluate(async ({ peerError }) => {
    await window.__TAURI_BROWSER_TEST__.emitEvent('runtime-service-status', {
      running: {}, lastError: 'instance failed to restore its checkpoint',
    })
    window.__TAURI_BROWSER_TEST__.releaseCheckpointHydration({}, peerError)
  }, { peerError })
  const warning = page.getByText(hydrationWarning, { exact: true })
  await expect(warning).toBeVisible()
  await page.evaluate(async () => {
    await window.__TAURI_BROWSER_TEST__.emitEvent('runtime-service-status', {
      running: {}, lastError: 'instance failed to restore its checkpoint',
    })
    await window.__TAURI_BROWSER_TEST__.emitEvent('checkpoint-status', {
      instance_id: 'browser-test-instance', phase: 'ready', routable: true,
    })
  })
  await expect(warning).toBeVisible()
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.emitEvent('runtime-service-status', {
    running: {}, lastError: null, checkpoints: {},
  }))
  await expect(warning).toHaveCount(0)
  await expect(page.getByText('background runtime: instance failed to restore its checkpoint', { exact: true })).toBeVisible()
})

test('late startup failures cannot restore warnings after checkpoint recovery', async ({ page }) => {
  await page.goto('/?scenario=checkpoint-hydration-delayed')
  await page.evaluate(async ({ peerError }) => {
    await window.__TAURI_BROWSER_TEST__.emitEvent('runtime-service-status', {
      running: {}, lastError: null, checkpoints: {},
    })
    window.__TAURI_BROWSER_TEST__.releaseCheckpointHydration({}, peerError)
    // Let the delayed IPC rejection and its UI update finish before checking absence.
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  }, { peerError })
  await expect(page.getByText(hydrationWarning, { exact: true })).toHaveCount(0)
  await page.evaluate(({ peerError }) => window.__TAURI_BROWSER_TEST__.emitEvent(
    'runtime-service-error', { error: peerError },
  ), { peerError })
  await expect(page.getByText(`background runtime: ${peerError}`, { exact: true })).toBeVisible()
})

test('late startup success cannot overwrite a newer checkpoint snapshot', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('lastTab', 'instances'))
  await page.goto('/?scenario=checkpoint-hydration-delayed')
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.emitEvent('runtime-service-status', {
    running: {}, lastError: null,
    checkpoints: {
      'browser-test-instance': {
        instance_id: 'browser-test-instance', phase: 'ready', routable: true,
        expected_pid: 1234, last_operation: 'restore', last_outcome: 'success',
        reason_code: 'none', message: '', updated_at: Date.now(),
      },
    },
  }))
  await expect(page.getByText('已就绪（文件已验证）', { exact: true })).toBeVisible()
  await page.evaluate(async () => {
    window.__TAURI_BROWSER_TEST__.releaseCheckpointHydration({
      'browser-test-instance': {
        instance_id: 'browser-test-instance', phase: 'restart_required', routable: false,
        expected_pid: 1234, last_operation: 'restore', last_outcome: 'failed',
        reason_code: 'none', message: '', updated_at: 1,
      },
    })
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  })
  await expect(page.getByText('已就绪（文件已验证）', { exact: true })).toBeVisible()
  await expect(page.getByText('恢复失败，等待新进程隔离', { exact: true })).toHaveCount(0)
})
