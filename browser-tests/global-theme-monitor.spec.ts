import { expect, test } from '@playwright/test'

test('vector monitoring keeps batch counts, queue and item rates in their own units', async ({ page }) => {
  await page.clock.setFixedTime(Date.now())
  await page.addInitScript(() => { localStorage.setItem('lastTab', 'perf'); localStorage.setItem('lang', 'zh-CN') })
  await page.goto('/?scenario=monitoring&workload=vector')
  const speed = page.locator('[data-reading=speed]')
  await expect(speed).toContainText('向量吞吐')
  await expect(speed).toContainText('12条/s')
  await expect(page.locator('.monitor-state')).toContainText('排队 2')
  await expect(page.locator('.monitor-records')).toContainText('最近批次')
  await expect(page.locator('.monitor-records tbody')).toContainText('12 条/s')
  await speed.click()
  await expect(page.locator('.monitor-detail')).not.toContainText('tok/s')
  await page.locator('.monitor-actions').getByRole('button', { name: '大屏模式' }).click()
  await expect(page.locator('[data-wall-rate=items]')).toContainText('12条/s')
  await page.getByRole('button', {name:'返回监控'}).click()
  await page.locator('[data-guide=perf-select] select').selectOption('browser-stopped-instance')
  await page.locator('.monitor-actions').getByRole('button', { name: '大屏模式' }).click()
  await page.getByRole('button', {name:'返回监控'}).click()
  await expect(page.locator('[data-guide=perf-select] select')).toHaveValue('browser-stopped-instance')
  await expect(page.locator('.monitor-state')).toContainText('已停止')
})

for (const dark of [false, true]) {
  test(`every workspace shares the ${dark ? 'dark' : 'light'} canvas, including guide, wall and routing`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.addInitScript(() => {
      localStorage.setItem('lastTab', 'instances')
      localStorage.setItem('lang', 'zh-CN')
    })
    await page.goto('/?scenario=docs-screenshots')
    await expect(page.locator('.instance-inspector')).toBeVisible()
    if ((await page.locator('html').evaluate((el) => el.classList.contains('dark'))) !== dark) {
      await page.getByRole('button', { name: dark ? '切换到深色模式' : '切换到明亮模式' }).click()
    }
    const canvas = dark ? 'rgb(14, 17, 19)' : 'rgb(249, 250, 251)'
    const surface = dark ? 'rgb(22, 26, 29)' : 'rgb(255, 255, 255)'
    const inset = dark ? 'rgb(29, 34, 38)' : 'rgb(243, 245, 246)'
    const nav = page.locator('[data-nav-id]')
    const tabs = await nav.evaluateAll((nodes) => nodes.map((n) => n.getAttribute('data-nav-id')!))
    expect(tabs).toHaveLength(13)
    for (const id of tabs) {
      await page.locator('[data-nav-id="' + id + '"]').click()
      await expect(page.locator('[data-workspace-scroll]')).toHaveCSS('background-color', canvas)
      if (id === 'engine') {
        await expect(page.locator('[data-workspace-scroll] .console-surface').first()).toHaveCSS(
          'background-color',
          surface,
        )
        await expect(page.locator('[data-workspace-scroll] .console-inset').first()).toHaveCSS(
          'background-color',
          inset,
        )
      } else if (id === 'guide') {
        const article = page.locator('.guide-content')
        await expect(article.locator('..')).toHaveCSS('background-color', surface)
        await expect(article.locator('../..')).toHaveCSS('background-color', canvas)
      } else if (id === 'proxy') {
        await page.getByRole('tab', { name: 'API Key', exact: true }).click()
        await expect(page.locator('.router-key-rows').locator('..')).toHaveCSS('background-color', surface)
      } else if (id === 'bigscreen') {
        await expect(page.locator('[data-guide="monitoring-wall"]')).toHaveCSS('background-color', canvas)
      } else if (id === 'perf') {
        await expect(page.locator('[data-reading=temperature]')).toHaveCSS('background-color', surface)
      }
    }
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
  })
}

for (const missing of [false, true]) {
  test(`live cards, session records and wall share ${missing ? 'missing' : 'measured'} sensors`, async ({
    page,
  }) => {
    await page.clock.setFixedTime(Date.now())
    await page.addInitScript(() => {
      localStorage.setItem('lastTab', 'perf')
      localStorage.setItem('lang', 'zh-CN')
    })
    await page.goto('/?scenario=monitoring' + (missing ? '&sensors=missing' : ''))
    const temperature = page.locator('[data-reading=temperature]')
    await expect(temperature).toContainText(missing ? '传感器未上报' : '48')
    await expect(page.locator('[data-reading=power]')).toContainText(missing ? '传感器未上报' : '36')
    await expect(page.locator('.monitor-tile')).toHaveCount(4)
    await temperature.click()
    await expect(page.locator('.monitor-detail')).toContainText('GPU 温度')
    if (missing) await expect(page.locator('.monitor-detail svg[role=img]')).toHaveCount(0)
    await page.locator('[data-reading=speed]').click()
    await page.getByRole('button', { name: '预填充速度', exact: true }).click()
    await expect(page.locator('.monitor-detail h3')).toContainText('预填充速度')
    await expect(page.locator('.monitor-records tbody tr')).toHaveCount(3)
    await page.locator('.monitor-records header button').click()
    await expect(page.locator('.monitor-records tbody tr')).toHaveCount(5)
    await page.locator('.monitor-records tbody button').first().click()
    await expect(page.locator('.monitor-record-detail')).toContainText('#784')
    await page.getByRole('button', { name: '历史会话', exact: true }).click()
    await expect(page.locator('.monitor-records')).toContainText('峰值显存')
    await page.locator('.monitor-actions').getByRole('button', { name: '大屏模式' }).click()
    await expect(page.locator('[data-wall-sensor=temperature]')).toContainText(
      missing ? '传感器未上报' : '48',
    )
    await expect(page.locator('[data-wall-rate]')).toContainText('25.8')
    await page.getByRole('button', { name: '返回监控' }).click()
    await expect(temperature).toBeVisible()
  })
}
