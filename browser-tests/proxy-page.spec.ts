import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('route switches expose current state and saving refreshes the enabled-rule count', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=proxy-routing')

  const routeSection = page.getByRole('heading', { name: '路由表' }).locator('xpath=ancestor::section[1]')
  const enabledMetric = page.getByText('已启用规则', { exact: true }).locator('..')
  const healthyMetric = page.getByText('当前健康路由', { exact: true }).locator('..')
  await expect(enabledMetric.locator('p').nth(1)).toHaveText('0')
  await expect(healthyMetric.locator('p').nth(1)).toHaveText('0')

  await routeSection.getByRole('button', { name: '添加路由' }).click()
  const routeSwitch = routeSection.getByRole('switch', { name: '路由启用状态' })
  await expect(routeSwitch).toBeChecked()
  await expect(routeSection.getByText('已启用', { exact: true })).toBeVisible()
  await expect(routeSection.getByText('待保存', { exact: true })).toBeVisible()
  await expect(routeSection.getByRole('combobox', { name: '目标' })).toHaveValue('browser-test-instance')
  await expect(page.getByRole('button', { name: '保存' })).toBeDisabled()
  await expect(routeSection.getByRole('button', { name: '测试路由' })).toBeDisabled()

  await routeSwitch.click()
  await expect(routeSwitch).not.toBeChecked()
  await expect(routeSection.getByText('已禁用', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '保存' })).toBeEnabled()

  await routeSwitch.click()
  await routeSection.getByRole('textbox', { name: '对外模型名' }).fill('public-browser-model')
  await expect(page.getByRole('button', { name: '保存' })).toBeEnabled()
  await page.getByRole('button', { name: '保存' }).click()

  await expect(page.getByText('代理配置已保存并生效')).toBeVisible()
  await expect(enabledMetric.locator('p').nth(1)).toHaveText('1')
  await expect(healthyMetric.locator('p').nth(1)).toHaveText('1')
  await expect(routeSection.getByText('当前命中', { exact: true })).toBeVisible()
  await expect(routeSection).toContainText('显式规则会遮蔽对应实例的对外别名')

  const savedRoute = await page.evaluate(() => {
    const call = [...window.__TAURI_BROWSER_TEST__.calls].reverse().find(item => item.command === 'save_proxy_config')
    const payload = call?.payload as { config?: { routes?: Array<Record<string, unknown>> } } | undefined
    return payload?.config?.routes?.[0]
  })
  expect(savedRoute).toMatchObject({
    enabled: true,
    model_alias: 'public-browser-model',
    target_instance_id: 'browser-test-instance',
  })
})

test('route health separates enabled rules from currently healthy failover targets', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=proxy-route-health')

  const enabledMetric = page.getByText('已启用规则', { exact: true }).locator('..')
  const healthyMetric = page.getByText('当前健康路由', { exact: true }).locator('..')
  await expect(enabledMetric.locator('p').nth(1)).toHaveText('2')
  await expect(healthyMetric.locator('p').nth(1)).toHaveText('1')

  const routeSection = page.getByRole('heading', { name: '路由表' }).locator('xpath=ancestor::section[1]')
  const rows = routeSection.locator('tbody tr')
  await expect(rows).toHaveCount(2)
  await expect(rows.nth(0)).toContainText('目标已停止')
  await expect(rows.nth(1)).toContainText('当前命中')

  await rows.nth(0).getByRole('button', { name: '测试路由' }).click()
  await expect(rows.nth(0)).toContainText('当前实际命中: Browser Parameter Regression')

  await rows.nth(1).getByRole('button', { name: '测试路由' }).click()
  await expect(rows.nth(1)).toContainText('测试通过，命中: Browser Parameter Regression')
})
