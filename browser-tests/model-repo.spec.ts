import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('a fresh configuration exposes the default model root and its scanned tree', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'model-repo')
  })
  await page.goto('/')

  await expect(page.locator('html')).toHaveAttribute(
    'data-tauri-browser-test',
    '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__',
  )
  await expect(page.getByText('1 个来源', { exact: true })).toBeVisible()
  const explorer = page.locator('[data-guide="model-search"]')
  await expect(explorer.getByText('C:\\browser-test\\models', { exact: true })).toBeVisible()
  await expect(
    explorer.getByText('Qwen Browser Test Q8_0.gguf', { exact: true }),
  ).toBeVisible()
})

test('an empty legacy root list is repaired before a manual scan', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'model-repo')
  })
  await page.goto('/?scenario=empty-model-roots')

  await expect(page.getByText('0 个来源', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '扫描模型', exact: true }).click()

  await expect(page.getByText('1 个来源', { exact: true })).toBeVisible()
  const explorer = page.locator('[data-guide="model-search"]')
  await expect(explorer.getByText('C:\\browser-test\\models', { exact: true })).toBeVisible()
  await expect(explorer.getByText('Qwen Browser Test Q8_0.gguf', { exact: true })).toBeVisible()
  await expect.poll(() => page.evaluate(() => {
    const calls = window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'scan_models')
    return calls[calls.length - 1]?.payload
  })).toEqual({ paths: ['C:\\browser-test\\models'] })
})
