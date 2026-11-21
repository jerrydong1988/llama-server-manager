import { expect, test, type Page } from '@playwright/test'

async function openInstanceList(page: Page) {
  await page.setViewportSize({ width: 1280, height: 600 })
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=instance-order-filter')
}

test('saving then returning reveals the selected instance and retains the list filters', async ({ page }) => {
  await openInstanceList(page)
  const search = page.getByPlaceholder(/搜索实例/)
  await search.fill('Group')
  const engine = page.locator('.instance-list-container select')
  const [engineId] = await engine.selectOption({ index: 1 })
  const beta = page.locator('.instance-list-row').filter({ hasText: 'Group Beta' })
  await beta.getByText('Group Beta', { exact: true }).click()
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  await expect(page.locator('#config-page-actions')).toContainText('Group Beta')
  await page.locator('[data-config-field="temp"] input').fill('0.72')
  const actions = page.locator('[data-config-floating-actions]')
  await actions.locator('[data-config-floating-save]').click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  await actions.getByRole('button', { name: '返回实例列表' }).click()
  await expect(beta).toHaveAttribute('aria-selected', 'true')
  await expect(beta).toBeInViewport({ ratio: 0.99 })
  await expect(search).toHaveValue('Group')
  await expect(engine).toHaveValue(engineId)
  await expect(page.locator('.instance-list-row')).toHaveCount(2)
  // Return before the lazy dashboard module loads, as on a first visit.
  const dashboardModule = '**/components/Dashboard/Dashboard.tsx*'
  let releaseDashboard!: () => void
  const pending = new Promise<void>(resolve => { releaseDashboard = resolve })
  await page.route(dashboardModule, async route => { await pending; await route.continue() })
  try {
    await Promise.all([page.waitForRequest(dashboardModule), page.locator('[data-nav-id="dashboard"]').click()])
    await page.locator('[data-nav-id="instances"]').click()
  } finally {
    releaseDashboard()
  }
  await expect(beta).toHaveAttribute('aria-selected', 'true')
  await expect(beta).toBeInViewport({ ratio: 0.99 })
  await page.locator('.instance-list-row').getByText('Group Alpha', { exact: true }).click()
  await page.locator('[data-nav-id="config"]').click()
  await expect(page.locator('#config-page-actions')).toContainText('Group Alpha')
})

test('sidebar return selects the row opened from its menu and reveals it after its port changes', async ({ page }) => {
  await openInstanceList(page)
  const beta = page.locator('.instance-list-row').filter({ hasText: 'Group Beta' })
  await expect(beta).toHaveAttribute('aria-selected', 'false')
  await beta.locator('summary').click()
  await beta.getByRole('button', { name: '配置参数', exact: true }).click()
  await expect(page.locator('#config-page-actions')).toContainText('Group Beta')
  await page.locator('[data-nav-id="instances"]').click()
  await expect(beta).toHaveAttribute('aria-selected', 'true')
  await expect(beta).toBeInViewport({ ratio: 0.99 })

  const search = page.getByPlaceholder(/搜索实例/)
  await search.fill('18083')
  const engine = page.locator('.instance-list-container select')
  const [engineId] = await engine.selectOption({ index: 1 })
  await page.locator('[data-nav-id="config"]').click()
  await page.locator('[data-config-field="port"] input').fill('18222')
  const header = page.locator('#config-page-actions')
  await header.getByRole('button', { name: '保存配置' }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  await header.getByRole('button', { name: '返回实例列表' }).click()
  await expect(search).toHaveValue('')
  await expect(engine).toHaveValue(engineId)
  await expect(beta).toHaveAttribute('aria-selected', 'true')
  await expect(beta).toContainText('18222')
})

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
