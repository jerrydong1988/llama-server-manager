import { expect, test } from '@playwright/test'

for (const width of [1280, 1920]) {
  test(`last instance menu remains clickable outside its list at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1200 })
    await page.addInitScript(() => {
      localStorage.setItem('lang', 'zh-CN')
      localStorage.setItem('lastTab', 'instances')
    })
    await page.goto('/')
    const row = page.locator('.instance-list-row').last()
    await row.locator('summary').click()
    const configure = row.getByRole('button', { name: '配置参数', exact: true })
    await expect.poll(() => configure.evaluate(element => {
      const box = element.getBoundingClientRect()
      return element.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2))
    })).toBe(true)
    await configure.click()
    await expect(page.getByRole('button', { name: '托管参数模式', exact: false })).toBeVisible()
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
  })
}
