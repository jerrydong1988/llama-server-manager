import { expect, test, type Page } from '@playwright/test'

async function openKeys(page: Page, count: number) {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=proxy-routing')
  const refresh = page.locator('[data-guide="proxy-overview"]').getByRole('button', { name: '刷新' })
  await expect(refresh).toBeEnabled({ timeout: 15_000 })
  await page.evaluate(async (n) => {
    const api = (window as any).__TAURI_INTERNALS__
    const config = await api.invoke('get_proxy_config')
    config.api_keys = Array.from({ length: n }, (_, i) => ({
      id: `client-${i + 1}`,
      name: i === 0 ? 'WorkBuddy' : `Client ${String(i + 1).padStart(2, '0')}`,
      key: `fixture-credential-${i}`,
      enabled: (i + 1) % 4 !== 0,
      scopes: ['inference', 'discovery'],
      requests_per_minute: 60,
      max_concurrent_requests: 2,
      daily_token_budget: 0,
      monthly_token_budget: 0,
      daily_token_limit: 0,
      monthly_token_limit: 0,
      quota_default_output_tokens: 32768,
    }))
    await api.invoke('save_proxy_config', { config })
  }, count)
  await refresh.click()
  await page.getByRole('tab', { name: 'API Key', exact: true }).click()
  await expect(page.getByRole('button', { name: '编辑 WorkBuddy' })).toBeVisible()
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('many keys stay paginated and retain list position across editing and filters', async ({ page }, info) => {
  await page.setViewportSize({ width: 1600, height: 1000 })
  await openKeys(page, 24)
  const panel = page.getByRole('tabpanel', { name: 'API Key', exact: true })
  const rows = panel.locator('tbody tr')
  await expect(rows).toHaveCount(8)
  await expect(rows.locator('input')).toHaveCount(0)
  await expect(panel.getByText('1–8 项，共 24 项')).toBeVisible()
  await page.screenshot({ path: info.outputPath('key-list-desktop.png') })
  await panel.getByRole('button', { name: '下一页' }).click()
  await panel.getByRole('button', { name: '编辑 Client 12' }).click()
  await expect(panel.getByRole('switch', { name: '启用此 Key' })).not.toBeChecked()
  await page.screenshot({ path: info.outputPath('key-editor-desktop.png') })
  await panel.getByRole('spinbutton', { name: '每分钟请求数', exact: true }).fill('125')
  await panel.getByRole('button', { name: '保存此 Key' }).click()
  await expect(panel.getByText('9–16 项，共 24 项')).toBeVisible()
  await expect(rows.filter({ hasText: 'Client 12' })).toContainText('125')
  await panel.getByRole('searchbox').fill('cLIenT')
  await panel.getByRole('button', { name: /^已停用/ }).click()
  await expect(rows).toHaveCount(6)
  await panel.getByRole('searchbox').fill('absent')
  await expect(panel.getByText('没有符合条件的 Key')).toBeVisible()
  await panel.getByRole('button', { name: '清除筛选' }).click()
  await page.setViewportSize({ width: 800, height: 850 })
  await panel.getByRole('button', { name: '编辑 WorkBuddy' }).click()
  await expect(panel.getByRole('heading', { name: '编辑 API Key' })).toBeVisible()
  await expect(panel.getByRole('table')).toBeHidden()
  await page.getByRole('button', { name: '切换到明亮模式' }).click()
  await expect(panel.getByRole('spinbutton', { name: '每分钟请求数', exact: true })).toHaveCSS('background-color', 'rgb(255, 255, 255)')
  await expect(panel.getByRole('button', { name: '保存此 Key' })).toBeInViewport()
  await page.screenshot({ path: info.outputPath('key-editor-narrow-light.png') })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await panel.getByRole('button', { name: '取消', exact: true }).last().click()
  await expect(rows.first()).toBeVisible()
})

test('stale key edits cannot overwrite an external change and drafts survive a conflict', async ({ page }) => {
  await openKeys(page, 1)
  await page.getByRole('button', { name: '编辑 WorkBuddy' }).click()
  const rpm = page.getByRole('spinbutton', { name: '每分钟请求数', exact: true })
  await rpm.fill('135')
  await page.evaluate(async () => {
    const api = (window as any).__TAURI_INTERNALS__
    const config = await api.invoke('get_proxy_config')
    config.api_keys[0].requests_per_minute = 200
    await api.invoke('save_proxy_config', { config, expectedRevision: config.config_revision })
  })
  await page.getByRole('button', { name: '保存此 Key' }).click()
  await expect(page.getByRole('alert')).toContainText(/changed|冲突|修改/i)
  await expect(rpm).toHaveValue('135')
  await page.locator('[data-guide="proxy-overview"]').getByRole('button', { name: '刷新' }).click()
  await page.getByRole('button', { name: '保存此 Key' }).click()
  await expect(page.getByRole('alert')).toContainText('此 Key 已被其他操作修改')
  await page.getByRole('button', { name: '取消', exact: true }).last().click()
  await page.getByRole('button', { name: '放弃修改' }).click()
  await page.getByRole('button', { name: '编辑 WorkBuddy' }).click()
  await expect(rpm).toHaveValue('200')
  await page.getByRole('button', { name: '密钥', exact: true }).click()
  await page.getByRole('button', { name: '轮换密钥' }).click()
  await expect(page.getByRole('textbox', { name: 'API Key（至少 16 字符）' })).toHaveValue(/^lsm_/)
  await page.getByRole('button', { name: '保存此 Key' }).click()
  await page.getByRole('button', { name: '编辑 WorkBuddy' }).click()
  await page.getByRole('button', { name: '密钥', exact: true }).click()
  await expect(page.getByRole('button', { name: '复制新 API Key' })).toHaveCount(0)
  await page.getByRole('button', { name: '删除 API Key', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('最后一个 Key')
  await page.getByRole('button', { name: '确认删除此 Key' }).click()
  await expect(page.getByRole('table', { name: '调用方密钥' }).locator('tbody tr')).toHaveCount(0)
})
