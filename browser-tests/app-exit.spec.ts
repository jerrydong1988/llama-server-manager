import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('active workloads offer verified background handoff as the primary exit action', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
  })
  await page.goto('/')
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.emitEvent(
    'proxy-exit-confirmation-requested',
    { backgroundServiceMode: false },
  ))

  const dialog = page.getByRole('dialog', { name: '后台实例或路由仍在运行' })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('button', { name: '启用后台并退出' })).toBeVisible()
  await expect(dialog.getByRole('button', { name: '保持托盘运行' })).toBeVisible()
  await expect(dialog.getByRole('button', { name: '停止实例与路由并退出' })).toBeVisible()

  await dialog.getByRole('button', { name: '启用后台并退出' }).click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.some(call => call.command === 'enable_background_and_quit')
  ))).toBe(true)
})

test('failed handoff keeps the UI open and exposes the backend error', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
  })
  await page.goto('/?scenario=background-detach-error')
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.emitEvent(
    'proxy-exit-confirmation-requested',
    { backgroundServiceMode: false },
  ))

  const dialog = page.getByRole('dialog', { name: '后台实例或路由仍在运行' })
  await dialog.getByRole('button', { name: '启用后台并退出' }).click()
  await expect(dialog.getByRole('alert')).toContainText('Browser test background handoff failed')
  await expect(dialog.getByRole('button', { name: '启用后台并退出' })).toBeEnabled()
})

test('tray handoff failures reopen a recoverable error dialog', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
  })
  await page.goto('/?scenario=background-detach-error')
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.emitEvent(
    'background-detach-failed',
    { error: 'Runtime receipt mismatch' },
  ))

  const dialog = page.getByRole('alertdialog', { name: '后台接管失败' })
  await expect(dialog).toContainText('Runtime receipt mismatch')
  await dialog.getByRole('button', { name: '重新验证并退出' }).click()
  await expect(dialog).toContainText('Browser test background handoff failed')
})

test('destructive background shutdown stays in routing settings behind confirmation', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=background-runtime-active')

  await page.getByRole('button', { name: '停止后台实例与路由' }).click()
  const dialog = page.getByRole('alertdialog', { name: '确认停止全部后台工作负载？' })
  await expect(dialog).toBeVisible()
  await dialog.getByRole('button', { name: '确认停止全部' }).click()

  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.some(call => call.command === 'stop_background_runtime')
  ))).toBe(true)
  await expect(page.getByText('独立后台运行、托管实例和路由已停止。')).toBeVisible()
})
