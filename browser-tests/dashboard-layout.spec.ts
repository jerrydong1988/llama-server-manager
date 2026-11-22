import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => { await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]') })

test('footer panels share column edges and a baseline as download history grows', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('lang', 'zh-CN'); localStorage.setItem('lastTab', 'dashboard') })
  await page.setViewportSize({ width: 2560, height: 1397 })
  await page.goto('/?scenario=monitoring')
  const footer = page.getByTestId('dashboard-footer')
  const cards = footer.locator(':scope > aside')
  const bounds = () => cards.evaluateAll(nodes => nodes.map(node => node.getBoundingClientRect().toJSON()))
  await expect(footer.getByText('暂无下载任务')).toBeVisible()
  for (const count of [0, 1, 4]) {
    if (count) {
      await page.evaluate(async n => {
        const mock = window.__TAURI_BROWSER_TEST__
        mock.downloadQueueOverride = [{
          id: 'footer-history', repo_id: 'layout/models', source: 'huggingface', save_dir: 'models',
          added_at: Date.now(), status: 'completed',
          files: Array.from({ length: n }, (_,i) => ({
            task_id: 'file-' + i, name: 'model-' + i + '.gguf', path: 'model-' + i + '.gguf',
            size: 1024, downloaded: 1024, file_type: 'model', status: 'completed',
          })),
        }]
        await mock.emitEvent('management-config-changed', { operation: 'downloads.settings', revision: 0 })
      }, count)
      await expect(footer.getByTitle(/^model-\d\.gguf$/)).toHaveCount(count)
    }
    const [downloads, inventory] = await bounds()
    expect(downloads.top).toBe(inventory.top)
    expect(downloads.bottom).toBe(inventory.bottom)
    for (const [card, above] of [[downloads, page.getByTestId('dashboard-instances')], [inventory, page.getByTestId('dashboard-secondary')]] as const) {
      const box = (await above.boundingBox())!
      expect(card.left).toBeCloseTo(box.x, 0)
      expect(card.width).toBeCloseTo(box.width, 0)
    }
  }
  await page.setViewportSize({ width: 1280, height: 900 })
  const [left, right] = await bounds()
  expect(left.bottom).toBe(right.bottom)
  expect(left.width).toBeCloseTo(right.width, 0)
  await page.setViewportSize({ width: 860, height: 900 })
  const [upper, lower] = await bounds()
  expect(lower.top).toBeGreaterThan(upper.bottom)
  expect(lower.left).toBe(upper.left)
  expect(lower.width).toBe(upper.width)
})

for (const lang of ['zh-CN', 'en-US']) {
  test(`dashboard prioritizes resources and instance controls in ${lang}`, async ({ page }, testInfo) => {
    await page.addInitScript(language => { localStorage.setItem('lang', language); localStorage.setItem('lastTab', 'dashboard') }, lang)
    await page.setViewportSize({ width: 1920, height: 1400 })
    await page.goto('/?scenario=monitoring')
    const dashboard = page.locator('[data-guide="dashboard"]')
    const header = page.getByTestId('dashboard-header')
    const resources = page.getByTestId('dashboard-resources')
    const actions = page.getByTestId('dashboard-actions')
    const instances = page.getByTestId('dashboard-instances')
    const router = page.getByTestId('router-overview')
    await expect(resources).toContainText('44%')
    await expect(page.getByTestId('dashboard-secondary').getByTestId('router-overview')).toBeVisible()
    const boxes = await Promise.all([header, resources, actions, instances, router].map(el => el.boundingBox()))
    expect(boxes.every(Boolean)).toBe(true)
    const [h, r, a, i, route] = boxes.map(b => b!)
    expect(h.y + h.height).toBeLessThan(r.y)
    expect(r.y + r.height).toBeLessThan(a.y)
    expect(a.y + a.height).toBeLessThan(i.y)
    expect(route.x).toBeGreaterThan(i.x)
    expect(route.y).toBeGreaterThan(r.y + r.height)
    const themeButton = page.getByRole('button', { name: lang === 'zh-CN' ? '切换到明亮模式' : 'Switch to light mode' })
    await page.mouse.move(10, 10)
    await dashboard.screenshot({ path: testInfo.outputPath(`dashboard-${lang}-dark.png`) })
    await themeButton.click()
    await dashboard.screenshot({ path: testInfo.outputPath(`dashboard-${lang}-light.png`) })
    await page.setViewportSize({ width: 860, height: 1400 })
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    const narrowInstances = await instances.boundingBox()
    const narrowRouter = await router.boundingBox()
    expect(narrowRouter!.y).toBeGreaterThan(narrowInstances!.y + narrowInstances!.height)
    await page.screenshot({ path: testInfo.outputPath(`dashboard-${lang}-narrow.png`) })
    await router.getByRole('button').click()
    await expect(page.getByRole('tab', { name: lang === 'zh-CN' ? '路由与实例' : 'Routes & instances' })).toHaveAttribute('aria-selected','true')
  })
}
