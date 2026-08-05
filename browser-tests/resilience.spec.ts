import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('automatic instance startup preserves the configured stagger without duplicates', async ({ page }) => {
  await page.goto('/?scenario=auto-start-stagger')

  await expect.poll(async () => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'start_server').length
  )), { timeout: 10_000 }).toBe(2)

  const starts = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls
    .filter(call => call.command === 'start_server')
    .map(call => ({ at: call.at, payload: call.payload as { instanceId?: string } })))
  expect(starts.map(call => call.payload.instanceId)).toEqual([
    'browser-test-instance',
    'browser-auto-start-second',
  ])
  expect(starts[1].at - starts[0].at).toBeGreaterThanOrEqual(2_900)
})

test('instance start reports an immediate transient state while the backend is pending', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=delayed-instance-start')

  const start = page.getByRole('button', { name: '启动', exact: true }).first()
  await start.click()
  await expect(page.getByRole('button', { name: '启动中...', exact: true }).first()).toBeDisabled()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'start_server').length
  ))).toBe(1)

  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releaseStart())
  await expect(page.getByRole('button', { name: '停止', exact: true }).first()).toBeEnabled()
})

test('a transient updater failure is visible and a manual retry discovers the update', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('lang', 'zh-CN'))
  await page.goto('/?scenario=updater-retry')

  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.updaterCheckCount)).toBe(1)
  const retry = page.getByRole('button', { name: '检查更新' })
  await expect(retry).toHaveAttribute('title', /temporarily unavailable/)
  await retry.click()

  await expect(page.getByRole('button', { name: '安装可用更新' })).toContainText('v2.9.37')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.updaterCheckCount)).toBe(2)
})

test('available updater completes installation and requests a relaunch', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('lang', 'zh-CN'))
  await page.goto('/?scenario=updater-install')

  await page.getByRole('button', { name: '安装可用更新' }).click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.some(call => call.command === 'plugin:updater|download_and_install')
  ))).toBe(true)
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.some(call => call.command === 'plugin:process|restart')
  ))).toBe(true)
})

test('updater installation failure remains recoverable and reports the error', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('lang', 'zh-CN'))
  await page.goto('/?scenario=updater-install-failure')

  const install = page.getByRole('button', { name: '安装可用更新' })
  await install.click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.some(call => (
      call.command === 'plugin:dialog|message'
      && JSON.stringify(call.payload).includes('browser test updater download failed')
    ))
  ))).toBe(true)
  await expect(install).toBeEnabled()
})

test('paused downloads restore from the backend snapshot and can resume', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'downloads')
  })
  await page.goto('/?scenario=download-resume')

  await expect(page.getByText('browser-model.gguf', { exact: true }).first()).toBeVisible()
  await page.getByTitle('▶ 继续').click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.some(call => call.command === 'resume_download_task')
  ))).toBe(true)
})

test('cluster workers load from persistence and a connection test refreshes their status', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'cluster')
  })
  await page.goto('/?scenario=cluster-worker')

  await expect(page.getByText('Browser Cluster Worker', { exact: true }).first()).toBeVisible()
  await page.getByRole('button', { name: '测试连接' }).click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.some(call => call.command === 'test_worker')
  ))).toBe(true)
  await expect(page.getByText('在线', { exact: true }).first()).toBeVisible()
})
