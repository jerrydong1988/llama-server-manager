import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('interactive guide waits for stable layout and exposes only the current navigation item', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'guide')
  })
  await page.goto('/?scenario=docs-screenshots')

  await page.getByRole('button', { name: '开始交互式引导' }).click()

  await expect(page.locator('[data-guide="dashboard"]')).toHaveClass(/driver-active-element/)
  await expect(page.locator('[data-nav-id][aria-current="page"]')).toHaveCount(1)
  await expect(page.locator('[data-nav-id="dashboard"]')).toHaveAttribute('aria-current', 'page')
  await expect(page.locator('.driver-popover')).toBeVisible()
  await expect(page.locator('.driver-popover-title')).toHaveText('系统总览')

  await page.locator('.driver-popover-close-btn').click()
  await expect(page.locator('[data-nav-id="guide"]')).toHaveAttribute('aria-current', 'page')
})
