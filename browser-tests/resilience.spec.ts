import { expect, test, type Page } from '@playwright/test'

async function openEngineManager(page: Page, scenario?: string) {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'engine')
  })
  await page.goto(scenario ? `/?scenario=${scenario}` : '/')
  await expect(page.locator('html')).toHaveAttribute(
    'data-tauri-browser-test',
    '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__',
  )
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('engine qualification turns fail-closed evidence into a passing report', async ({ page }) => {
  await openEngineManager(page, 'engine-qualification')

  await page.getByRole('button', { name: '实例管理', exact: true }).click()
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  await expect(page.getByTestId('engine-qualification-notice')).toBeVisible()
  await expect(page.getByTestId('engine-qualification-notice')).toContainText('实例启动会被安全阻止')
  await page.getByRole('button', { name: '引擎管理', exact: true }).click()

  const panel = page.getByTestId('engine-qualification-panel')
  await expect(panel.getByText('未认证', { exact: true })).toBeVisible()
  await expect(page.getByTestId('qualification-no-report')).toBeVisible()
  await page.getByTestId('run-engine-qualification').click()

  await expect(panel.getByText('已通过', { exact: true })).toBeVisible()
  await expect(page.getByTestId('qualification-report')).toBeVisible()
  await expect(page.getByTestId('qualification-report').getByText(/^通过 ·/)).toHaveCount(5)
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls
      .filter(call => call.command === 'qualify_engine')
      .map(call => call.payload)
  ))).toEqual([{
    engineId: 'browser-test-engine',
    modelId: 'browser-test-model',
  }])

  await page.getByRole('button', { name: '参数配置', exact: true }).click()
  await expect(page.getByTestId('engine-qualification-notice')).toBeHidden()
})

test('engine rename commits after a prior Escape cancellation', async ({ page }) => {
  await openEngineManager(page)

  const renameButton = page.getByTitle('重命名引擎')
  await renameButton.click()
  await page.getByRole('textbox', { name: '重命名引擎' }).press('Escape')
  await expect(page.getByRole('textbox', { name: '重命名引擎' })).toBeHidden()

  await renameButton.click()
  const input = page.getByRole('textbox', { name: '重命名引擎' })
  await input.fill('Linux Engine Renamed')
  await page.getByText('Vulkan', { exact: true }).last().click()

  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls
      .filter(call => call.command === 'rename_engine')
      .map(call => call.payload)
  ))).toEqual([{ id: 'browser-test-engine', name: 'Linux Engine Renamed' }])
  await expect(page.getByText('Linux Engine Renamed', { exact: true }).first()).toBeVisible()
})

test('engine rename waits for Linux IME composition before Enter commits', async ({ page }) => {
  await openEngineManager(page)

  await page.getByTitle('重命名引擎').click()
  const input = page.getByRole('textbox', { name: '重命名引擎' })
  await input.fill('中文引擎')
  await input.dispatchEvent('keydown', { key: 'Enter', code: 'Enter', isComposing: true })

  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'rename_engine').length
  ))).toBe(0)
  await expect(input).toBeVisible()

  await input.press('Enter')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls
      .filter(call => call.command === 'rename_engine')
      .map(call => call.payload)
  ))).toEqual([{ id: 'browser-test-engine', name: '中文引擎' }])
  await expect(page.getByText('中文引擎', { exact: true }).first()).toBeVisible()
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

test('an explicitly infeasible resource plan blocks launch before persistence or process start', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=resource-plan-infeasible')

  await page.getByRole('button', { name: '启动', exact: true }).first().click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'plan_instance_resources').length
  ))).toBe(1)
  expect(await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'start_server').length
  ))).toBe(0)
  await expect(page.getByRole('button', { name: '启动', exact: true }).first()).toBeEnabled()
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

test('Secure Agent workers load from persistence and an authenticated test refreshes their status', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'cluster')
  })
  await page.goto('/?scenario=cluster-worker')

  await expect(page.getByText('Browser Secure Agent', { exact: true }).first()).toBeVisible()
  await page.getByRole('button', { name: '测试连接' }).click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.some(call => call.command === 'test_worker_agent')
  ))).toBe(true)
  await expect(page.getByText('在线', { exact: true }).first()).toBeVisible()
})
