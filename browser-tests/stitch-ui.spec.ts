import { expect, test, type Page } from '@playwright/test'

async function openWorkspace(page: Page, tab = 'dashboard') {
  await page.addInitScript(initialTab => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', initialTab)
  }, tab)
  await page.goto('/?scenario=monitoring')
  await expect(page.locator('html')).toHaveAttribute('data-tauri-browser-test', '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__')
  await expect(page.locator(`[data-nav-id="${tab}"]`)).toHaveAttribute('aria-current', 'page')
}

async function toggleTheme(page: Page) {
  await page.locator('.app-topbar').getByRole('button').last().click()
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

for (const width of [1024, 1280, 1600]) {
  test(`desktop pages fit their scroll container at ${width}px in both themes`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 })
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await openWorkspace(page)
    for (const theme of ['dark', 'light']) {
      if (theme === 'light') await toggleTheme(page)
      for (const id of ['dashboard', 'instances', 'model-repo', 'downloads', 'logs', 'perf', 'proxy', 'storage']) {
        await page.locator(`[data-nav-id="${id}"]`).click()
        await expect(page.locator('.app-main')).toHaveAttribute('data-page-id', id)
        await expect(page.locator('.app-content .ui-panel').first()).toBeVisible()
        const bounds = await page.locator('.app-content').evaluate(element => ({
          client: element.clientWidth,
          scroll: element.scrollWidth,
        }))
        expect(bounds.scroll, `${id} ${theme}`).toBeLessThanOrEqual(bounds.client + 1)
      }
      await page.locator('[data-nav-id="instances"]').click()
      await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
      await expect(page.getByRole('textbox', { name: '参数搜索' })).toBeVisible()
      const overflow = await page.locator('.app-content').evaluate(element => element.scrollWidth - element.clientWidth)
      expect(overflow, `config ${theme}`).toBeLessThanOrEqual(1)
    }
    expect(errors).toEqual([])
  })
}

test('sampling sliders retain extended values, precise input, and drafts across theme changes', async ({ page }) => {
  await openWorkspace(page, 'instances')
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  const temperature = page.locator('[data-config-field="temp"]')
  const topK = page.locator('[data-config-field="top_k"]')
  const repeat = page.locator('[data-config-field="repeat_penalty"]')

  await topK.getByRole('spinbutton').fill('450')
  await repeat.getByRole('spinbutton').fill('3.7')
  await temperature.getByRole('spinbutton').fill('0.65')
  await expect(temperature.getByRole('slider')).toHaveValue('0.65')
  await expect(topK.getByRole('slider')).toHaveAttribute('max', '450')
  await expect(repeat.getByRole('slider')).toHaveAttribute('max', '3.7')
  await toggleTheme(page)
  await expect(topK.getByRole('spinbutton')).toHaveValue('450')
  await expect(repeat.getByRole('spinbutton')).toHaveValue('3.7')
  await expect(temperature.getByRole('spinbutton')).toHaveValue('0.65')
  await expect(temperature.getByRole('slider')).toHaveValue('0.65')
  await topK.getByRole('slider').focus()
  await topK.getByRole('slider').press('ArrowLeft')
  await expect(topK.getByRole('spinbutton')).toHaveValue('449')
  await temperature.getByRole('spinbutton').press('Control+s')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.top_k)).toBe(449)
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.repeat_penalty)).toBe(3.7)
})

test('model cards preserve selection and search when switching views or themes', async ({ page }) => {
  await openWorkspace(page, 'model-repo')
  await page.getByRole('button', { name: '模型卡片', exact: true }).click()
  const grid = page.locator('[data-model-grid]')
  await grid.getByRole('button', { name: /Vision Ambiguous/ }).click()
  await expect(grid.getByRole('button', { name: /Vision Ambiguous/ })).toHaveAttribute('aria-pressed', 'true')
  await toggleTheme(page)
  await expect(grid.getByRole('button', { name: /Vision Ambiguous/ })).toHaveAttribute('aria-pressed', 'true')
  await page.getByRole('button', { name: '目录列表', exact: true }).click()
  await page.getByRole('button', { name: '模型卡片', exact: true }).click()
  await expect(grid.getByRole('button', { name: /Vision Ambiguous/ })).toHaveAttribute('aria-pressed', 'true')
  await page.locator('[data-guide="model-search"] input').fill('Vision')
  await expect(grid.getByRole('button')).toHaveCount(1)
  await expect(grid).toContainText('Vision Ambiguous')
})

test('interface fonts load locally and the theme uses opaque component surfaces', async ({ page }) => {
  await openWorkspace(page)
  await page.evaluate(() => document.fonts.ready)
  const fonts = await page.evaluate(() => ({
    geist: document.fonts.check('500 13px Geist', 'Llama'),
    mono: document.fonts.check('400 12px "JetBrains Mono"', '128'),
  }))
  expect(fonts).toEqual({ geist: true, mono: true })
  for (const surface of ['rgb(13, 28, 45)', 'rgb(255, 255, 255)']) {
    await expect(page.locator('.ui-panel').first()).toHaveCSS('background-color', surface)
    await expect(page.locator('.ui-panel').first()).toHaveCSS('backdrop-filter', 'none')
    await toggleTheme(page)
  }
})
