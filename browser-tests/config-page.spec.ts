import { expect, test, type Page } from '@playwright/test'

const qwenProjectorPath = 'C:\\browser-test\\models\\mmproj-Qwen-BF16.gguf'

async function openConfiguration(page: Page, scenario?: string) {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto(scenario ? `/?scenario=${scenario}` : '/')
  await expect(page.locator('html')).toHaveAttribute(
    'data-tauri-browser-test',
    '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__',
  )
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  await expect(page.getByRole('textbox', { name: '参数搜索' })).toBeVisible()
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('opening an instance config keeps React hook order stable (issue #5)', async ({ page }) => {
  const pageErrors: string[] = []
  page.on('pageerror', error => pageErrors.push(error.message))

  await openConfiguration(page)
  await expect(page.locator('[data-config-field="temp"]')).toBeVisible()
  expect(pageErrors).toEqual([])
})

test('search navigation, change review, emission preview, and save use the mock backend', async ({ page }) => {
  await openConfiguration(page)

  const search = page.getByRole('textbox', { name: '参数搜索' })
  const temperature = page.locator('[data-config-field="temp"]')
  const temperatureInput = temperature.locator('input')
  await search.fill('--temp')
  await expect(temperature).toHaveAttribute('data-config-search-match', 'true')
  await search.press('Enter')
  await expect(temperature).toHaveAttribute('data-config-search-current', 'true')
  await expect(temperatureInput).toBeFocused()

  await temperatureInput.fill('0.7')
  await expect(temperature.locator('[data-config-status="changed"]')).toBeVisible()
  await expect(temperature).toHaveAttribute('data-config-emitted', 'true')

  const modelsAutoload = page.locator('[data-config-field="models_autoload"]')
  await expect(modelsAutoload.getByRole('switch')).toHaveAttribute('aria-checked', 'false')
  await expect(modelsAutoload).toHaveAttribute('data-config-emitted', 'true')

  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')

  const generated = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated)
  expect(generated?.command).toContain('--temp')
  expect(generated?.command).toContain('0.7')
  expect(generated?.command).toContain('--no-models-autoload')
  expect(generated?.emittedOverrideKeys).toContain('temp')
  expect(generated?.emittedOverrideKeys).toContain('models_autoload')
})

test('parameter intent remains explicit at a default value and only inheritance removes it', async ({ page }) => {
  await openConfiguration(page)

  const temperature = page.locator('[data-config-field="temp"]')
  const input = temperature.locator('input')
  await input.fill('0.8')
  await expect(temperature).toHaveAttribute('data-config-source', 'explicit')
  await expect(temperature).toHaveAttribute('data-config-emitted', 'true')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command.join(' ') ?? ''))
    .toContain('--temp 0.8')

  await temperature.getByRole('button', { name: /参数说明/ }).click()
  await expect(page.getByText('0.8', { exact: true }).last()).toBeVisible()
  await page.getByRole('button', { name: '恢复引擎默认', exact: true }).click()

  await expect(temperature).toHaveAttribute('data-config-source', 'inherited')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .not.toContain('--temp')
})

test('automatic numeric modes and the unified loading mode produce unambiguous commands', async ({ page }) => {
  await openConfiguration(page)

  const threads = page.locator('[data-config-field="threads"]')
  await expect(threads).toHaveAttribute('data-config-source', 'inherited')
  await threads.getByRole('combobox').selectOption('manual')
  await expect(threads).toHaveAttribute('data-config-source', 'explicit')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .toContain('--threads')
  await threads.getByRole('combobox').selectOption('inherit')
  await expect(threads).toHaveAttribute('data-config-source', 'inherited')

  const contextSize = page.locator('[data-config-field="ctx_size"]')
  await contextSize.getByRole('combobox').selectOption('manual')
  await expect(contextSize).toHaveAttribute('data-config-source', 'explicit')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .toContain('-c')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.emittedOverrideKeys ?? []))
    .toEqual(expect.arrayContaining(['ctx_size']))
  await contextSize.getByRole('button', { name: /参数说明/ }).click()
  await page.getByRole('button', { name: '恢复引擎默认', exact: true }).click()
  await expect(contextSize).toHaveAttribute('data-config-source', 'inherited')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .not.toContain('-c')

  const loadMode = page.locator('[data-config-field="load_mode"]')
  const loadModeSelect = loadMode.getByRole('combobox')
  await expect(loadModeSelect).toHaveValue('')
  await loadModeSelect.selectOption('none')
  await expect(loadMode).toHaveAttribute('data-config-source', 'explicit')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .toEqual(expect.arrayContaining(['--load-mode', 'none']))
  await loadModeSelect.selectOption('mmap')
  await expect(loadModeSelect).toHaveValue('mmap')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .toEqual(expect.arrayContaining(['--load-mode', 'mmap']))
  await expect.poll(() => page.evaluate(() => {
    const command = window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []
    return command.filter(argument => ['--mlock', '--no-mmap', '--direct-io'].includes(argument))
  })).toEqual([])
})

test('source-confirmed multimodal projector is emitted without a mismatch warning', async ({ page }) => {
  await openConfiguration(page, 'multimodal-match')

  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-emitted', /mmproj_path/)
  await expect(page.getByText('projector 与主模型的来源元数据存在冲突', { exact: true })).toHaveCount(0)

  const generated = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated)
  expect(generated?.command).toContain('--mmproj')
  expect(generated?.command).toContain(qwenProjectorPath)
})

test('conflicting multimodal projector source is surfaced as a validation warning', async ({ page }) => {
  await openConfiguration(page, 'multimodal-mismatch')

  await expect(page.getByText('projector 与主模型的来源元数据存在冲突；请重新选择与该模型配套的多模态投影器', { exact: true })).toBeVisible()
})

test('backend command generation failure blocks persistence', async ({ page }) => {
  await openConfiguration(page, 'command-error')
  const before = await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'generate_server_command').length
  ))

  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'generate_server_command').length
  ))).toBeGreaterThan(before)
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '0')
})

test('an empty managed alias becomes a visible safe API identifier', async ({ page }) => {
  await openConfiguration(page, 'empty-alias')

  const alias = page.locator('[data-config-field="alias"]')
  await expect(alias.locator('input')).toHaveValue('Browser Parameter Regression')
  await expect(alias).toHaveAttribute('data-config-emitted', 'true')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .toContain('-a')
  const generated = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated)
  const aliasIndex = generated?.command.indexOf('-a') ?? -1
  expect(aliasIndex).toBeGreaterThanOrEqual(0)
  expect(generated?.command[aliasIndex + 1]).toBe('Browser Parameter Regression')
})
