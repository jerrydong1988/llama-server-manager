import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('storage maintenance separates cleanup, restart scheduling, revocation, and inventory-only output', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'storage')
  })
  await page.goto('/?scenario=storage-maintenance')

  await expect(page.locator('h1').getByText('存储维护', { exact: true })).toBeVisible()
  await expect(page.locator('[data-storage-group="private-scratch"]')).toContainText('原子写入与检查点暂存')
  await expect(page.locator('[data-storage-group="webview-cache"]')).toContainText('Cookie、Local Storage、IndexedDB')
  await expect(page.getByText('C:\\operator\\slots', { exact: true })).toBeVisible()

  page.once('dialog', dialog => dialog.accept())
  await page.locator('[data-storage-action="cleanup-updater-staging"]').click()
  await expect(page.getByText(/已移除 0 个项目.*有 1 个项目失败/)).toBeVisible()
  await expect(page.getByText('Refusing linked updater staging directory', { exact: true })).toBeVisible()

  page.once('dialog', dialog => dialog.accept())
  await page.locator('[data-storage-action="webview-schedule"]').click()
  await expect(page.getByText('已安排在下次重启时清理 WebView 缓存。', { exact: true })).toBeVisible()

  page.once('dialog', dialog => dialog.accept())
  await page.locator('[data-storage-action="revoke-directory"]').first().click()
  await expect(page.getByText('目录访问权已撤销。', { exact: true })).toBeVisible()

  const commands = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.map(call => call.command))
  expect(commands).toContain('cleanup_storage_group')
  expect(commands).toContain('schedule_webview_cache_cleanup')
  expect(commands).toContain('revoke_authorized_directory')
  expect(commands).not.toContain('delete_managed_local_file')
})
