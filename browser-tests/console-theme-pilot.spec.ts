import { expect, test, type Page } from '@playwright/test'

async function openPilot(page: Page, scenario = 'docs-screenshots') {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=' + scenario)
  await expect(page.locator('.instance-list-row').first()).toBeVisible()
}

for (const dark of [false, true]) {
  test(`shell, selected instance, input and save share the ${dark ? 'dark' : 'light'} palette`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await openPilot(page)
    if (await page.locator('html').evaluate(el => el.classList.contains('dark')) !== dark) {
      await page.getByRole('button', { name: dark ? '切换到深色模式' : '切换到明亮模式' }).click()
    }
    const action = dark ? 'rgb(25, 59, 49)' : 'rgb(207, 241, 230)'
    const surface = dark ? 'rgb(22, 26, 29)' : 'rgb(255, 255, 255)'
    const selected = dark ? 'rgb(26, 48, 40)' : 'rgb(236, 250, 243)'
    await expect(page.locator('.shell-header')).toHaveCSS('background-color', surface)
    await expect(page.locator('[data-nav-id=instances]')).toHaveCSS('background-color', action)
    await expect(page.locator('[data-guide=instance-create]')).toHaveCSS('background-color', action)
    await expect(page.locator('.instance-list-row[aria-selected=true]')).toHaveCSS('background-color', selected)
    await page.locator('.instance-inspector').getByRole('button', { name: '配置参数', exact: true }).click()
    await expect(page.locator('[data-guide=config-save]')).toHaveCSS('background-color', action)
    const actions = page.locator('#config-page-actions .console-action-row button')
    await expect(actions).toHaveCount(4)
    const bounds = await actions.evaluateAll(buttons => buttons.map(b => ({
      height: b.getBoundingClientRect().height, y: b.getBoundingClientRect().y,
      font: getComputedStyle(b).fontSize, icon: b.querySelector('svg')?.getBoundingClientRect().width,
    })))
    expect(bounds.every(b => b.height === 36 && b.y === bounds[0].y && b.font === '13px' && b.icon === 16)).toBe(true)
    const port = page.locator('[data-config-field="port"] input')
    await expect(port).toHaveCSS('background-color', surface)
    await port.fill('18503')
    await expect(page.locator('#config-page-actions').getByRole('status')).toContainText('本次改动')
    await page.locator('.config-jump').selectOption('config-advanced-custom')
    await expect(page.locator('#config-advanced-custom')).toBeInViewport()
    await page.locator('[data-nav-id="model-repo"]').click()
    await expect(page.locator('[data-guide="model-search"]')).toBeVisible()
    await expect(page.locator('[data-workspace-scroll]')).toHaveCSS('background-color', dark ? 'rgb(14, 17, 19)' : 'rgb(249, 250, 251)')
    await expect(page.locator('[data-guide="model-directories"] .console-btn').first()).toBeVisible()
  })
}

test('compact shell preserves workspace height and supports named navigation', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 768 })
  await openPilot(page)
  const scroll = page.locator('[data-workspace-scroll]')
  expect((await scroll.boundingBox())!.height).toBeGreaterThan(650)
  const picker = page.getByRole('combobox', { name: '页面导航' })
  await picker.selectOption('config')
  await expect(page.locator('[data-workspace=config]')).toBeVisible()
  await page.setViewportSize({ width: 650, height: 768 })
  await picker.selectOption('instances')
  await expect(page.getByRole('button', { name: '创建实例' })).toBeInViewport()
  await page.getByRole('searchbox').fill('no-such-instance')
  await page.getByRole('button', { name: '清除筛选' }).click()
  await expect(page.locator('.instance-list-row').first()).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
})

test('Gufo saves the active draft from the keyboard and reports invalid input', async ({ page }) => {
  await openPilot(page, 'gufo')
  await page.locator('.instance-inspector').getByRole('button', { name: '配置参数', exact: true }).click()
  const budget = page.getByLabel('思考预算（Token 数）', { exact: true })
  await budget.fill('-2')
  await expect(budget).toHaveAttribute('aria-invalid', 'true')
  await page.keyboard.press('Control+s')
  await expect(page.getByRole('alert')).toBeVisible()
  await budget.fill('256')
  await page.keyboard.press('Control+s')
  await expect(page.getByRole('status')).toContainText('配置已保存')
  await expect.poll(() => page.evaluate(() =>
    window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.gufo.reasoning_budget,
  )).toBe(256)
})

for (const dark of [false, true]) {
  test(`data lists distinguish hover, selection, default engine and offline workers in ${dark ? 'dark' : 'light'} mode`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await openPilot(page)
    if (await page.locator('html').evaluate(el => el.classList.contains('dark')) !== dark) {
      await page.getByRole('button', { name: dark ? '切换到深色模式' : '切换到明亮模式' }).click()
    }
    const selected = dark ? 'rgb(26, 48, 40)' : 'rgb(236, 250, 243)'
    const hover = dark ? 'rgb(37, 46, 48)' : 'rgb(234, 240, 238)'
    const surface = dark ? 'rgb(22, 26, 29)' : 'rgb(255, 255, 255)'
    const ink = dark ? 'rgb(238, 241, 243)' : 'rgb(34, 30, 31)'
    const instance = page.locator('.instance-list-row').filter({ hasText: 'Embedding Mini' })
    await instance.getByText('Embedding Mini', { exact: true }).hover()
    await expect(instance).toHaveCSS('background-color', hover)
    await instance.getByText('Embedding Mini', { exact: true }).click()
    await expect(instance).toHaveCSS('background-color', selected)
    await page.mouse.move(0, 0)
    await expect(instance).toHaveCSS('background-color', selected)
    await page.locator('.instance-list-container').screenshot({ path: testInfo.outputPath('instance-rows.png') })

    await page.locator('[data-nav-id=engine]').click()
    const engines = page.locator('.console-list-row')
    const vulkan = engines.filter({ has: page.getByRole('button', { name: 'Vulkan', exact: true }) })
    const engineName = vulkan.getByRole('button', { name: 'Vulkan', exact: true })
    await engineName.hover()
    await expect(vulkan).toHaveCSS('background-color', hover)
    await engineName.focus()
    await page.keyboard.press('Space')
    await expect(engineName).toHaveAttribute('aria-pressed', 'true')
    await expect(vulkan).toHaveCSS('background-color', selected)
    await expect(engineName).toHaveCSS('color', ink)
    const defaultEngine = engines.filter({ has: page.locator('span[aria-label="默认引擎"]') })
    await expect(defaultEngine).toContainText('CUDA 12.4')
    await expect(defaultEngine).toHaveAttribute('data-selected', 'false')
    await page.mouse.move(0, 0)
    await vulkan.locator('..').screenshot({ path: testInfo.outputPath('engine-rows.png') })

    await page.locator('[data-nav-id=cluster]').click()
    const worker = page.locator('.console-list-row').filter({ hasText: 'NAS Compute Node' })
    await worker.getByText('NAS Compute Node', { exact: true }).hover()
    await expect(worker).toHaveCSS('background-color', hover)
    await expect(worker.getByText('离线', { exact: true })).toBeVisible()
    const details = worker.getByRole('button', { name: '查看详情' })
    await details.click()
    await expect(worker).toHaveCSS('background-color', selected)
    await page.mouse.move(0, 0)
    await expect(worker.getByRole('button', { name: '收起详情' })).toHaveAttribute('aria-expanded', 'true')
    await expect(worker.getByText('NAS Compute Node', { exact: true })).toHaveCSS('color', ink)
    await worker.locator('..').screenshot({ path: testInfo.outputPath('worker-rows.png') })
    await worker.getByRole('button', { name: '收起详情' }).click()
    await page.mouse.move(0, 0)
    await expect(worker).toHaveCSS('background-color', surface)

    await page.locator('[data-nav-id="model-repo"]').click()
    const model = page.locator('[data-guide="model-search"] button[data-selected]').last()
    await model.click()
    await expect(model).toHaveCSS('background-color', selected)
    await expect(model).toHaveAttribute('aria-pressed', 'true')
    await expect(model).toHaveCSS('border-radius', '0px')
    expect(await model.evaluate(el => {
      const viewport = el.closest('.model-tree-viewport')!
      return el.getBoundingClientRect().left === viewport.getBoundingClientRect().left
        && el.getBoundingClientRect().width === viewport.clientWidth
    })).toBe(true)
    const folder = page.locator('.model-tree-viewport button[aria-expanded]').first()
    await folder.click()
    await expect(folder).toHaveAttribute('aria-expanded', 'false')
    await expect(model).toHaveCount(0)
    await folder.click()
    await expect(model).toBeVisible()
    await page.locator('[data-guide=model-search] input').fill('absent-model-7732')
    await expect(page.locator('.model-tree-viewport')).toContainText('没有匹配的文件或目录')
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
  })
}
