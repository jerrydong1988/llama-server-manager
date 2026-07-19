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
