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

test('cluster exposes only the authenticated Secure Agent enrollment path', async ({ page }) => {
  await openTab(page, 'cluster')

  await expect(page.locator('[data-guide="cluster-agent"]')).toBeVisible()
  await expect(page.getByText('0 Secure Agents', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '扫描局域网', exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: '添加 Worker', exact: true })).toHaveCount(0)
  const legacyCalls = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.filter(call => (
    ['scan_workers_tcp', 'add_worker', 'test_worker'].includes(call.command)
  )))
  expect(legacyCalls).toEqual([])
})

test('secure Worker Agent enrollment uses pinned file metadata and compute stays fail-closed', async ({ page }) => {
  await openTab(page, 'cluster', 'cluster-add-worker')

  await page.getByRole('button', { name: '安全 Agent', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: '注册安全 Worker Agent' })
  const inputs = dialog.locator('input')
  await inputs.nth(0).fill('Secure GPU Worker')
  await inputs.nth(1).fill('worker.example.net')
  await inputs.nth(2).fill('7443')
  await inputs.nth(3).fill('worker.example.net')
  await inputs.nth(4).fill('7444')
  await inputs.nth(5).fill('worker.example.net')
  await inputs.nth(6).fill('C:\\secure\\worker-agent.crt')
  await inputs.nth(7).fill('C:\\secure\\worker-agent.token')
  await inputs.nth(8).fill('50152')
  await dialog.getByRole('button', { name: '注册 Agent', exact: true }).click()

  await expect(page.getByText('Secure GPU Worker', { exact: true }).first()).toBeVisible()
  const startButton = page.getByTitle(/Secure Agent 计算暂不可用/)
  await expect(startButton).toBeDisabled()
  const lifecycleCalls = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.filter(call => (
    ['start_worker_agent', 'stop_worker_agent'].includes(call.command)
  )))
  expect(lifecycleCalls).toEqual([])

  const enrollment = await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.find(call => call.command === 'enroll_worker_agent')?.payload
  )) as { enrollment?: Record<string, unknown> } | undefined
  expect(enrollment?.enrollment).toMatchObject({
    controlHost: 'worker.example.net',
    tlsServerName: 'worker.example.net',
    tlsCertPath: 'C:\\secure\\worker-agent.crt',
    tokenPath: 'C:\\secure\\worker-agent.token',
  })
  expect(enrollment?.enrollment).not.toHaveProperty('token')
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
