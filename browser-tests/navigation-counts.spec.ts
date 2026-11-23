import { expect, test } from '@playwright/test'

for (const dark of [true, false]) {
  test(`navigation counts active transfers and running instances in ${dark ? 'dark' : 'light'} mode`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.addInitScript(() => {
      localStorage.setItem('lang', 'zh-CN')
      localStorage.setItem('lastTab', 'instances')
    })
    await page.goto('/?scenario=docs-screenshots', { waitUntil: 'domcontentloaded' })
    const downloads = page.locator('[data-nav-id=downloads]')
    const instances = page.locator('[data-nav-id=instances]')
    const transfers = downloads.locator('.shell-nav-count')
    const running = instances.locator('.shell-nav-count')
    await expect(transfers).toHaveText('1') // The other download is paused.
    await expect(running).toHaveText('1')
    if (!dark) await page.getByRole('button', { name: '切换到明亮模式' }).click()
    await expect(downloads).toHaveAccessibleName('下载管理')
    await expect(downloads).toHaveAccessibleDescription('进行中 1')
    await page.evaluate(async () => {
      const emit = window.__TAURI_BROWSER_TEST__.emitEvent
      await emit('server-started', {
        instanceId: 'browser-stopped-instance',
        pid: 4791,
        port: 18082,
        host: '127.0.0.1',
        command: 'test',
      })
      await emit('download-paused', {
        taskId: 'docs-download-task-active',
        runId: 'docs-download-run-active',
        version: 3,
        source: 'huggingface',
        repoId: 'Qwen/Qwen3-14B-GGUF',
        fileName: 'Qwen3-14B-Q4_K_M.gguf',
      })
    })
    await expect(running).toHaveText('2')
    await expect(transfers).toHaveCount(0)
    await page.setViewportSize({ width: 900, height: 768 })
    await expect(running).toBeVisible()
    expect(await running.evaluate((el) => getComputedStyle(el, '::after').content)).toBe('"2"')
    await page.setViewportSize({ width: 560, height: 768 })
    await expect(page.locator('.shell-page-picker option[value=instances]')).toContainText('2')
    await page.evaluate(async () => {
      for (const instanceId of ['browser-test-instance', 'browser-stopped-instance']) {
        await window.__TAURI_BROWSER_TEST__.emitEvent('server-stopped', {
          instanceId,
          expected: true,
          exitCode: 0,
        })
      }
    })
    await expect(running).toHaveCount(0)
    await expect(page.locator('.shell-page-picker option[value=instances]')).toHaveText('实例管理')
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
  })
}
