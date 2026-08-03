import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('routing page documents OpenAI and Anthropic endpoints and compatibility boundaries', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=proxy-routing')

  const compatibility = page.getByRole('heading', { name: 'API 兼容入口' }).locator('xpath=ancestor::section[1]')
  await expect(compatibility).toContainText('OpenAI')
  await expect(compatibility).toContainText('Anthropic')
  await expect(compatibility).toContainText('/v1/chat/completions')
  await expect(compatibility).toContainText('/v1/responses')
  await expect(compatibility).toContainText('/v1/messages')
  await expect(compatibility).toContainText('/v1/messages/count_tokens')
  await expect(compatibility).toContainText('/v1/models')
  await expect(compatibility).toContainText('/slots')
  await expect(compatibility).toContainText('/ready')
  await expect(compatibility).toContainText('/metrics')
  await expect(compatibility).toContainText('--jinja')
  await expect(compatibility).toContainText('32 MiB')
})

test('route switches expose current state and saving refreshes runtime health', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=proxy-routing')

  const routeSection = page.getByRole('heading', { name: '路由表' }).locator('xpath=ancestor::section[1]')
  const healthyMetric = page.getByText('健康路由', { exact: true }).locator('..')
  await expect(healthyMetric.locator('p').nth(1)).toHaveText('1/1')

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
  await expect(healthyMetric.locator('p').nth(1)).toHaveText('1/1')
  await expect(routeSection.getByText('调度池', { exact: true })).toBeVisible()
  await expect(routeSection).toContainText('显式规则会遮蔽对应实例的对外别名')

  const savedRoute = await page.evaluate(() => {
    const call = [...window.__TAURI_BROWSER_TEST__.calls].reverse().find(item => item.command === 'save_proxy_config')
    const payload = call?.payload as { config?: { routes?: Array<Record<string, unknown>> } } | undefined
    return payload?.config?.routes?.[0]
  })
  expect(savedRoute).toMatchObject({
    enabled: true,
    weight: 1,
    max_concurrent_requests: 0,
    model_alias: 'public-browser-model',
    target_instance_id: 'browser-test-instance',
  })
})

test('production scheduling and scoped API keys round-trip through the settings UI', async ({ page }) => {
  await page.clock.install()
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=proxy-routing')

  const accessControl = page.getByRole('heading', { name: '访问控制与浏览器安全' }).locator('xpath=ancestor::section[1]')
  await expect(accessControl).toContainText('网页来源 → 调用身份 → 模型映射')
  await expect(accessControl).toContainText('Key 不绑定某个 Origin，也不绑定某条路由')
  await expect(accessControl).toContainText('当前不支持“一个 Key 只能调用某个模型”的逐路由授权')
  await expect(accessControl).toContainText('http://localhost:3000')
  await expect(page.getByText('旧版单一 API Key（可选）')).toHaveCount(0)

  await page.getByRole('combobox', { name: '调度策略' }).selectOption('weighted')
  await expect(page.getByRole('switch', { name: '已启用' })).toBeChecked()
  await page.getByRole('button', { name: '添加 API Key' }).click()
  const keyInput = page.getByRole('textbox', { name: 'API Key（至少 16 字符）' })
  await expect(keyInput).toHaveAttribute('type', 'password')
  const generatedKey = await keyInput.inputValue()
  expect(generatedKey).toMatch(/^lsm_[0-9a-f]{32}$/)
  await page.getByRole('button', { name: '显示新 API Key（10 秒）' }).click()
  await expect(keyInput).toHaveAttribute('type', 'text')
  await expect(keyInput).toHaveValue(generatedKey)
  await page.clock.fastForward(10_000)
  await expect(keyInput).toHaveAttribute('type', 'password')
  await expect(page.getByRole('button', { name: '显示新 API Key（10 秒）' })).toBeVisible()
  await expect(page.getByRole('button', { name: '复制新 API Key' })).toBeVisible()
  const rpmInput = page.getByRole('spinbutton', { name: '每分钟请求数', exact: true })
  await expect(rpmInput).toHaveValue('0')

  const [nameBox, keyBox, rpmBox, revealBox, copyBox, enabledBox, removeBox] = await Promise.all([
    accessControl.getByRole('textbox', { name: '名称', exact: true }).boundingBox(),
    keyInput.boundingBox(),
    rpmInput.boundingBox(),
    accessControl.getByRole('button', { name: '显示新 API Key（10 秒）' }).boundingBox(),
    accessControl.getByRole('button', { name: '复制新 API Key' }).boundingBox(),
    accessControl.getByRole('switch', { name: '已启用' }).boundingBox(),
    accessControl.getByRole('button', { name: '删除 API Key' }).boundingBox(),
  ])
  for (const box of [nameBox, keyBox, rpmBox, revealBox, copyBox, enabledBox, removeBox]) expect(box).not.toBeNull()
  expect(Math.abs(nameBox!.y - keyBox!.y)).toBeLessThanOrEqual(1)
  expect(Math.abs(rpmBox!.y - keyBox!.y)).toBeLessThanOrEqual(1)
  const keyCenterY = keyBox!.y + keyBox!.height / 2
  for (const [control, box] of [
    ['reveal', revealBox!],
    ['copy', copyBox!],
    ['enabled', enabledBox!],
    ['remove', removeBox!],
  ] as const) {
    expect(Math.abs(box.y + box.height / 2 - keyCenterY), `${control} control should share the input centerline`).toBeLessThanOrEqual(1)
  }
  await page.getByRole('textbox', { name: /允许的 CORS Origin/ }).fill('https://app.example.com')
  await page.getByRole('button', { name: '保存' }).click()

  await expect(page.getByText('已不可逆哈希保存，无法显示原文；如已遗失，请输入新值轮换。')).toBeVisible()
  await expect(page.getByRole('button', { name: '显示新 API Key（10 秒）' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: '复制新 API Key' })).toHaveCount(0)
  const savedConfig = await page.evaluate(() => {
    const call = [...window.__TAURI_BROWSER_TEST__.calls].reverse().find(item => item.command === 'save_proxy_config')
    return (call?.payload as { config?: Record<string, unknown> } | undefined)?.config
  })
  expect(savedConfig).toMatchObject({
    routing_strategy: 'weighted',
    strict_model_routing: true,
    max_concurrent_requests: 64,
    queue_timeout_ms: 1000,
    cors_allowed_origins: ['https://app.example.com'],
    api_keys: [{
      name: 'API Key 1',
      scopes: ['inference', 'discovery'],
      requests_per_minute: 0,
    }],
  })
})

test('route health separates enabled rules from currently healthy failover targets', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=proxy-route-health')

  const healthyMetric = page.getByText('健康路由', { exact: true }).locator('..')
  await expect(healthyMetric.locator('p').nth(1)).toHaveText('1/2')

  const routeSection = page.getByRole('heading', { name: '路由表' }).locator('xpath=ancestor::section[1]')
  const rows = routeSection.locator('tbody tr')
  await expect(rows).toHaveCount(2)
  await expect(rows.nth(0)).toContainText('目标已停止')
  await expect(rows.nth(1)).toContainText('调度池')

  await rows.nth(0).getByRole('button', { name: '测试路由' }).click()
  await expect(rows.nth(0)).toContainText('当前实际命中: Browser Parameter Regression')

  await rows.nth(1).getByRole('button', { name: '测试路由' }).click()
  await expect(rows.nth(1)).toContainText('测试通过，命中: Browser Parameter Regression')

  await page.evaluate(() => {
    window.__TAURI_BROWSER_TEST__.failProxyStatus = true
    window.__TAURI_BROWSER_TEST__.failProxyTargets = true
    window.__TAURI_BROWSER_TEST__.failRuntimeStatus = true
  })
  await expect(healthyMetric.locator('p').nth(1)).toHaveText('—', { timeout: 7_000 })
  await expect(rows.nth(0)).toContainText('目标状态未知')
  await expect(rows.nth(1)).toContainText('目标状态未知')
  const overview = page.getByRole('heading', { name: '实例路由' }).locator('xpath=ancestor::section[1]')
  await expect(overview.getByText('未知', { exact: true })).toBeVisible()

  await page.evaluate(() => {
    window.__TAURI_BROWSER_TEST__.failProxyStatus = false
    window.__TAURI_BROWSER_TEST__.failProxyTargets = false
    window.__TAURI_BROWSER_TEST__.failRuntimeStatus = false
  })
  await expect(healthyMetric.locator('p').nth(1)).toHaveText('1/2', { timeout: 7_000 })
  await expect(rows.nth(1)).toContainText('调度池')
  await expect(overview.getByText('运行中', { exact: true })).toBeVisible()
})

test('legacy empty route ids are repaired before row operations and save', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'proxy')
  })
  await page.goto('/?scenario=proxy-route-legacy-ids')

  const routeSection = page.getByRole('heading', { name: '路由表' }).locator('xpath=ancestor::section[1]')
  const rows = routeSection.locator('tbody tr')
  await expect(rows).toHaveCount(2)
  const switches = routeSection.getByRole('switch', { name: '路由启用状态' })
  await expect(switches).toHaveCount(2)
  await expect(switches.nth(0)).toBeChecked()
  await expect(switches.nth(1)).toBeChecked()

  await switches.nth(0).click()
  await expect(switches.nth(0)).not.toBeChecked()
  await expect(switches.nth(1)).toBeChecked()
  await page.getByRole('button', { name: '保存' }).click()

  const ids = await page.evaluate(() => {
    const call = [...window.__TAURI_BROWSER_TEST__.calls].reverse().find(item => item.command === 'save_proxy_config')
    const payload = call?.payload as { config?: { routes?: Array<{ id?: string }> } } | undefined
    return payload?.config?.routes?.map(route => route.id) ?? []
  })
  expect(ids).toHaveLength(2)
  expect(ids.every(Boolean)).toBe(true)
  expect(new Set(ids).size).toBe(2)
})
