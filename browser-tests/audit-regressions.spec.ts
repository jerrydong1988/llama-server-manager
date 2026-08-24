import { expect, test, type Page } from '@playwright/test'

async function openTab(page: Page, tab: string, scenario?: string) {
  await page.addInitScript(({ activeTab }) => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', activeTab)
  }, { activeTab: tab })
  await page.goto(scenario ? `/?scenario=${scenario}` : '/')
  await expect(page.locator('html')).toHaveAttribute(
    'data-tauri-browser-test',
    '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__',
  )
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('instance rename recovers after Escape and waits for IME composition', async ({ page }) => {
  await openTab(page, 'instances')

  const renameButton = page.getByTitle('重命名').first()
  await renameButton.click()
  await page.getByRole('textbox', { name: '重命名' }).press('Escape')
  await expect(page.getByRole('textbox', { name: '重命名' })).toBeHidden()

  await renameButton.click()
  const input = page.getByRole('textbox', { name: '重命名' })
  await input.fill('中文实例')
  await input.dispatchEvent('keydown', { key: 'Enter', code: 'Enter', isComposing: true })
  await expect(input).toBeVisible()
  expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.saveCount)).toBe(0)

  await input.press('Enter')
  await expect(page.getByText('中文实例', { exact: true }).first()).toBeVisible()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.name
  ))).toBe('中文实例')
})

test('completed download import merges and persists existing model roots', async ({ page }) => {
  const consoleErrors: string[] = []
  page.on('console', message => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  await openTab(page, 'downloads', 'download-resume')

  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.emitEvent('download-complete', {
    taskId: 'browser-download-task',
    runId: 'browser-download-run',
    version: 1,
    fileName: 'browser-model.gguf',
    repoId: 'browser/model',
    source: 'huggingface',
    remotePath: 'browser-model.gguf',
    downloaded: 1_024,
    total: 1_024,
    path: 'C:\\browser-test\\downloads\\browser\\model\\browser-model.gguf',
  }))
  await page.getByTitle('入库扫描').first().click()

  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.model_dirs))
    .toEqual(['C:\\browser-test\\models', 'C:\\browser-test\\downloads'])
  await expect.poll(() => page.evaluate(() => {
    const calls = window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'scan_models')
    return calls[calls.length - 1]?.payload
  })).toEqual({ paths: ['C:\\browser-test\\models', 'C:\\browser-test\\downloads'] })
  expect(consoleErrors.filter(message => message.includes('validateDOMNesting'))).toEqual([])
})

test('filtered instance reordering swaps adjacent visible instances', async ({ page }) => {
  await openTab(page, 'instances', 'instance-order-filter')

  await page.locator('input[placeholder*="搜索实例"]').fill('Group')
  await page.getByText('Group Beta', { exact: true }).click()
  await page.getByRole('button', { name: '上移', exact: true }).last().click()

  await expect.poll(() => page.evaluate(() => {
    const calls = window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'save_config')
    return (calls[calls.length - 1]?.payload as { instanceOrder?: string[] } | undefined)?.instanceOrder
  })).toEqual(['browser-group-beta', 'browser-hidden-instance', 'browser-test-instance'])
})

test('StrictMode keeps instance connection results mounted', async ({ page }) => {
  await openTab(page, 'instances', 'instance-connection')

  await page.getByRole('button', { name: '测试连接', exact: true }).last().click()
  await expect(page.getByText('HTTP 200', { exact: true }).first()).toBeVisible()
})

test('only the latest port availability response updates the create dialog', async ({ page }) => {
  await openTab(page, 'instances', 'port-check-race')

  await page.getByRole('button', { name: '创建实例', exact: true }).first().click()
  const portInput = page.locator('input[type="number"]').last()
  await portInput.fill('18090')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'check_port').length
  ))).toBe(1)
  await portInput.fill('18091')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'check_port').length
  ))).toBe(2)

  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releasePortCheck(18091, true))
  await expect(page.getByText('端口可用', { exact: true })).toBeVisible()
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releasePortCheck(18090, false))
  await expect(page.getByText('端口可用', { exact: true })).toBeVisible()
  await expect(page.getByText('端口已被占用', { exact: true })).toHaveCount(0)
})

test('a cancelled cluster scan cannot overwrite or stop its replacement', async ({ page }) => {
  await openTab(page, 'cluster', 'cluster-scan-race')

  await page.getByRole('button', { name: '扫描局域网', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'scan_workers_tcp').length
  ))).toBe(1)
  await page.getByRole('button', { name: '停止扫描', exact: true }).click()
  await page.getByRole('button', { name: '扫描局域网', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'scan_workers_tcp').length
  ))).toBe(2)

  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releaseWorkerScan([{
    id: 'old-worker', host: '192.168.1.10', port: 50052, name: 'Old Worker', origin: 'manual',
    devices: [], status: 'Offline', auto_discovered: false,
  }]))
  await expect(page.getByRole('button', { name: '停止扫描', exact: true })).toBeVisible()
  await expect(page.getByText('Old Worker', { exact: true })).toHaveCount(0)

  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releaseWorkerScan([{
    id: 'new-worker', host: '192.168.1.11', port: 50052, name: 'New Worker', origin: 'manual',
    devices: [], status: 'Online', auto_discovered: false,
  }]))
  await expect(page.getByText('New Worker', { exact: true }).first()).toBeVisible()
  await expect(page.getByRole('button', { name: '扫描局域网', exact: true })).toBeVisible()
})

test('a newly added cluster worker is immediately connection-tested', async ({ page }) => {
  await openTab(page, 'cluster', 'cluster-add-worker')

  await page.getByRole('button', { name: '添加 Worker', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: '添加 Worker' })
  const inputs = dialog.locator('input')
  await inputs.nth(0).fill('192.168.50.20')
  await inputs.nth(1).fill('50053')
  await inputs.nth(2).fill('Added Worker')
  await dialog.getByRole('button', { name: '保存', exact: true }).click()

  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'test_worker').length
  ))).toBe(1)
  await expect(page.getByText('Added Worker', { exact: true }).first()).toBeVisible()
  await expect(page.getByText('在线', { exact: true }).first()).toBeVisible()
})

test('download browsing ignores stale responses and composing Enter', async ({ page }) => {
  await openTab(page, 'downloads', 'download-browse-race')

  const browseButton = page.getByRole('button', { name: '浏览文件', exact: true })
  const repoInput = browseButton.locator('xpath=..').locator('input')
  await repoInput.fill('ime/repo')
  await repoInput.dispatchEvent('keydown', { key: 'Enter', code: 'Enter', isComposing: true })
  expect(await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'browse_modelscope').length
  ))).toBe(0)

  await repoInput.fill('old/repo')
  await repoInput.press('Enter')
  await repoInput.fill('new/repo')
  await repoInput.press('Enter')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'browse_modelscope').length
  ))).toBe(2)

  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releaseBrowse('new/repo', [{
    name: 'new-model.gguf', path: 'new-model.gguf', size: 1_024, file_type: 'model',
  }]))
  await expect(page.getByText('new-model.gguf', { exact: true })).toBeVisible()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'check_local_files').length
  ))).toBe(1)
  expect(await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'check_local_file').length
  ))).toBe(0)
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releaseBrowse('old/repo', [{
    name: 'old-model.gguf', path: 'old-model.gguf', size: 1_024, file_type: 'model',
  }]))
  await expect(page.getByText('new-model.gguf', { exact: true })).toBeVisible()
  await expect(page.getByText('old-model.gguf', { exact: true })).toHaveCount(0)

  await repoInput.fill('save-dir/repo')
  await repoInput.press('Enter')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'browse_modelscope').length
  ))).toBe(3)
  await page.locator('[data-guide="download-save-dir"] input').fill('C:\\browser-test\\new-downloads')
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releaseBrowse('save-dir/repo', [{
    name: 'stale-directory-model.gguf', path: 'stale-directory-model.gguf', size: 1_024, file_type: 'model',
  }]))
  await expect(page.getByText('stale-directory-model.gguf', { exact: true })).toHaveCount(0)
})

test('engine scan failures are visible and do not borrow model scan loading state', async ({ page }) => {
  await openTab(page, 'engine', 'engine-scan-error')

  const scanButton = page.locator('[data-guide="engine-scan"] button').first()
  await scanButton.click()
  await expect(page.getByText('engine directory unavailable', { exact: true })).toBeVisible()
  await expect(scanButton).toBeEnabled()
})
