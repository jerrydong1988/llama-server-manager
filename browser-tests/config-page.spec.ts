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

test('resource planning is visible, invalidates after edits, and refreshes before save', async ({ page }) => {
  await openConfiguration(page)

  const panel = page.locator('[data-guide="resource-plan"]')
  await expect(panel).toBeVisible()
  await expect(panel.getByText('可行', { exact: true })).toBeVisible()
  await expect(panel).toContainText('置信度: 中')
  await expect(panel).toContainText('系统内存')
  await expect(panel).toContainText('显存')
  await expect(panel).toContainText('131,072')
  await expect(panel).not.toContainText('C:\\browser-test')

  const beforeEdit = await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'plan_instance_resources').length
  ))
  await page.locator('[data-config-field="temp"] input').fill('0.7')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'plan_instance_resources').length
  ))).toBeGreaterThan(beforeEdit)
  await expect(panel.getByText('可行', { exact: true })).toBeVisible()

  const beforeSave = await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'plan_instance_resources').length
  ))
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'plan_instance_resources').length
  ))).toBeGreaterThan(beforeSave)
})

test('configuration revisions expose redacted history, known-good audit, and transactional rollback', async ({ page }) => {
  await openConfiguration(page)

  const panel = page.getByTestId('config-revision-panel')
  const deploymentIdentity = page.getByTestId('deployment-identity-status')
  await expect(deploymentIdentity).toContainText(/部署身份|Deployment Identity/)
  await expect(deploymentIdentity).toContainText(/已验证，可启动|Verified and ready/)
  await expect(panel).toBeVisible()
  await expect(panel.getByText('保存配置', { exact: true })).toBeVisible()
  await expect(panel.getByText('已设置（内容已隐藏）', { exact: true })).toBeVisible()
  await expect(panel).not.toContainText('historical-browser-secret')
  await expect(panel).not.toContainText('must-not-render')

  const baseline = panel.locator('[data-revision-id="revision-baseline-browser-test-instance"]')
  const baselineToggle = baseline.locator('button[aria-expanded]')
  await baseline.scrollIntoViewIfNeeded()
  await baseline.getByText('迁移基线', { exact: true }).click()
  await expect(baselineToggle).toHaveAttribute('aria-expanded', 'true')
  await baseline.getByRole('button', { name: '标记为已知良好', exact: true }).click()
  await expect(baseline.getByText('已知良好', { exact: true })).toBeVisible()
  await expect(panel).toContainText('已标记新的已知良好修订')

  await baseline.getByRole('button', { name: '回滚', exact: true }).click()
  await expect(page.getByRole('alertdialog', { name: '确认配置回滚' })).toBeVisible()
  await page.getByRole('button', { name: '确认回滚并创建新修订', exact: true }).click()

  await expect(panel.getByText('回滚生成', { exact: true })).toBeVisible()
  await expect(page.locator('[data-config-field="port"] input')).toHaveValue('18080')
  await expect(page.getByText('Browser Parameter Regression', { exact: true }).first()).toBeVisible()
})

test('stale configuration rollback is rejected without refreshing the editor to a false success state', async ({ page }) => {
  await openConfiguration(page, 'config-revision-stale')

  const panel = page.getByTestId('config-revision-panel')
  const baseline = panel.locator('[data-revision-id="revision-baseline-browser-test-instance"]')
  const baselineToggle = baseline.locator('button[aria-expanded]')
  await baseline.scrollIntoViewIfNeeded()
  await baseline.getByText('迁移基线', { exact: true }).click()
  await expect(baselineToggle).toHaveAttribute('aria-expanded', 'true')
  await baseline.getByRole('button', { name: '回滚', exact: true }).click()
  await page.getByRole('button', { name: '确认回滚并创建新修订', exact: true }).click()

  await expect(panel).toContainText('CONFIG_REVISION_STALE')
  await expect(page.locator('[data-config-field="port"] input')).toHaveValue('18081')
  await expect(panel.getByText('回滚生成', { exact: true })).toHaveCount(0)
})

test('deployment identity explains an incomplete fail-closed gate without exposing backend detail', async ({ page }) => {
  await openConfiguration(page, 'deployment-identity-incomplete')

  const status = page.getByTestId('deployment-identity-status')
  await expect(status).toContainText('尚未就绪')
  await expect(status).toContainText('ENGINE_QUALIFICATION_INCOMPLETE')
  await expect(status).toContainText('请刷新模型和引擎清单、完成引擎资格认证，并保存当前配置。')
  await expect(status).not.toContainText('browser mock detail')
})

test('deployment identity presents stale artifact evidence as fail-closed', async ({ page }) => {
  await openConfiguration(page, 'deployment-identity-stale')

  const status = page.getByTestId('deployment-identity-status')
  await expect(status).toContainText('尚未就绪')
  await expect(status).toContainText('DEPLOYMENT_MODEL_IDENTITY_STALE')
  await expect(status).not.toContainText('stale browser mock detail')
})

test('deployment identity presents legacy snapshots as fail-closed', async ({ page }) => {
  await openConfiguration(page, 'deployment-identity-legacy')

  const status = page.getByTestId('deployment-identity-status')
  await expect(status).toContainText('尚未就绪')
  await expect(status).toContainText('DEPLOYMENT_IDENTITY_INVALID')
  await expect(status).not.toContainText('legacy browser mock detail')
})

test('unsaved secret-bearing fields are redacted from the change review', async ({ page }) => {
  await openConfiguration(page)

  const secret = 'unsaved-browser-secret-must-not-render'
  await page.getByRole('textbox', { name: '参数搜索' }).fill('api key')
  const apiKey = page.locator('[data-config-field="api_key"] input')
  await apiKey.scrollIntoViewIfNeeded()
  await apiKey.fill(secret)

  await expect(page.getByText('api key', { exact: true }).last()).toBeVisible()
  await expect(page.getByTestId('config-revision-panel')).not.toContainText(secret)
  await expect(page.locator('body')).not.toContainText(secret)
  await expect(page.getByText('已设置（内容已隐藏）', { exact: true }).last()).toBeVisible()
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
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.lastGenerated?.command.join(' ') ?? ''
  ))).toContain('--temp 0.7')

  const preflightCalls = await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'generate_server_command').length
  ))
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'generate_server_command').length
  ))).toBe(preflightCalls)

  const generated = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated)
  expect(generated?.command).toContain('--temp')
  expect(generated?.command).toContain('0.7')
  expect(generated?.command).toContain('--no-models-autoload')
  expect(generated?.emittedOverrideKeys).toContain('temp')
  expect(generated?.emittedOverrideKeys).toContain('models_autoload')
})

test('Ctrl+S persists the active configuration draft through the validated save path', async ({ page }) => {
  await openConfiguration(page)

  const temperatureInput = page.locator('[data-config-field="temp"] input')
  await temperatureInput.fill('0.7')
  await temperatureInput.press('Control+s')

  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.temp
  ))).toBe(0.7)
})

test('Ctrl+Enter does not start an instance while a configuration input is focused', async ({ page }) => {
  await openConfiguration(page)

  const temperatureInput = page.locator('[data-config-field="temp"] input')
  await temperatureInput.focus()
  await temperatureInput.press('Control+Enter')

  expect(await page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'start_server').length
  ))).toBe(0)
})

test('configuration search and custom arguments wait for IME composition to finish', async ({ page }) => {
  await openConfiguration(page)

  const search = page.getByRole('textbox', { name: '参数搜索' })
  await search.fill('--temp')
  await search.dispatchEvent('keydown', { key: 'Enter', code: 'Enter', isComposing: true })
  await expect(page.locator('[data-config-search-current="true"]')).toHaveCount(0)
  await expect(search).toBeFocused()
  await search.press('Enter')
  await expect(page.locator('[data-config-field="temp"]')).toHaveAttribute('data-config-search-current', 'true')

  const customArgs = page.locator('[data-config-field="custom_args"]')
  await customArgs.scrollIntoViewIfNeeded()
  const nameInput = customArgs.getByPlaceholder('参数名称')
  await nameInput.fill('--custom-中文')
  await nameInput.dispatchEvent('keydown', { key: 'Enter', code: 'Enter', isComposing: true })
  await expect(nameInput).toHaveValue('--custom-中文')
  await expect(customArgs.getByText('--custom-中文', { exact: true })).toHaveCount(0)
  await nameInput.press('Enter')
  await expect(customArgs.getByText('--custom-中文', { exact: true })).toBeVisible()
})

test('floating config actions save without a long scroll and return to the top', async ({ page }) => {
  await openConfiguration(page)

  await page.locator('[data-config-field="temp"] input').fill('0.7')
  await page.locator('[data-config-field="custom_args"]').scrollIntoViewIfNeeded()

  const floatingActions = page.locator('[data-config-floating-actions]')
  await expect(floatingActions).toBeVisible()
  await floatingActions.locator('[data-config-floating-save]').click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')

  await floatingActions.locator('[data-config-back-to-top]').click()
  await expect(page.locator('#config-page-actions')).toBeInViewport()
  await expect(floatingActions).toBeHidden()
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
