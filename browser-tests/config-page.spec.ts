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

test('v0.7.0 controls support search, zero overrides, save, inheritance, and draft dependencies', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters')
  const search = page.getByRole('textbox', { name: '参数搜索' })
  const moe = page.locator('[data-config-field="moe_cache_mib"]')
  const batch = page.locator('[data-config-field="spec_draft_n_ubatch"]')
  const sampling = page.locator('[data-config-field="spec_draft_sampling"]')
  await search.fill('--moe-cache-mib')
  await expect(moe).toHaveAttribute('data-config-search-match', 'true')
  await expect(moe).toHaveAttribute('data-config-source', 'inherited')
  await moe.locator('input').fill('1024')
  await search.fill('--spec-draft-n-ubatch')
  await batch.locator('input').fill('128')
  await batch.locator('input').fill('0')
  await search.fill('--spec-draft-sampling')
  await sampling.getByRole('combobox').selectOption('probabilistic')
  await expect(sampling).toHaveAttribute('data-config-source', 'explicit')
  await expect(batch).toHaveAttribute('data-config-source', 'explicit')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command)).toContain('--spec-draft-sampling')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  const saved = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'])
  expect(saved).toMatchObject({ moe_cache_mib: 1024, spec_draft_n_ubatch: 0, spec_draft_sampling: 'probabilistic' })
  expect(saved.explicit_overrides).toEqual(expect.arrayContaining(['moe_cache_mib', 'spec_draft_n_ubatch', 'spec_draft_sampling']))
  await sampling.getByRole('button', { name: /参数说明/ }).click()
  await page.getByRole('button', { name: '恢复引擎默认', exact: true }).click()
  await expect(sampling).toHaveAttribute('data-config-source', 'inherited')
  await expect(sampling.getByRole('combobox')).toHaveValue('')
  await search.fill('--spec-type')
  await page.locator('[data-config-field="spec_type"]').getByRole('button', { name: '推测解码类型 (--spec-type)', exact: true }).click()
  await page.getByRole('listbox', { name: '推测解码类型 (--spec-type)', exact: true }).getByRole('button', { name: 'none', exact: true }).click()
  await search.fill('--spec-draft-n-ubatch')
  await expect(batch.locator('input')).toBeDisabled()
  await expect(batch).toHaveAttribute('data-config-source', 'inactive')
})

test('v0.7.0 parameters expose unsupported values on older engines', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters-old')
  const search = page.getByRole('textbox', { name: '参数搜索' })
  for (const [field, flag, value] of [
    ['moe_cache_mib', '--moe-cache-mib', '1024'],
    ['spec_draft_n_ubatch', '--spec-draft-n-ubatch', '128'],
    ['spec_draft_sampling', '--spec-draft-sampling', 'probabilistic'],
  ]) {
    await search.fill(flag)
    const control = page.locator(`[data-config-field="${field}"]`)
    if (field === 'spec_draft_sampling') await control.getByRole('combobox').selectOption(value)
    else await control.locator('input').fill(value)
    await expect(control).toHaveAttribute('data-config-source', 'unsupported')
  }
})

test('manual mode explains and persists the fallback instance port', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters')
  await page.getByRole('button', { name: /手动启动命令/ }).click()
  await page.getByRole('textbox', { name: '完整启动命令或可执行文件后的参数' }).fill('-m model.gguf')
  const port = page.locator('[data-config-field="port"] input')
  await expect(port).toHaveValue('18081')
  await port.fill('19091')
  await expect(page.getByText(/省略 --port 时会补入下方实例端口/)).toBeVisible()
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'])).toMatchObject({ launch_mode: 'manual', port: 19091, manual_command: '-m model.gguf' })
})

test('decimal parameter edits retain typed fractions', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters')
  const temp = page.locator('[data-config-field="temp"] input')
  await temp.fill('')
  await temp.pressSequentially('0.83')
  await expect(temp).toHaveValue('0.83')
})

test('all floating point controls accept fractions without step validation errors', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters')
  const fields = [
    'temp', 'top_p', 'repeat_penalty', 'min_p', 'presence_penalty', 'frequency_penalty',
    'mirostat_lr', 'mirostat_ent', 'xtc_probability', 'xtc_threshold', 'dynatemp_range',
    'dynatemp_exp', 'typical_p', 'dry_multiplier', 'dry_base', 'adaptive_target',
    'adaptive_decay', 'top_n_sigma', 'spec_draft_p_min', 'spec_draft_p_split',
    'rope_scale', 'rope_freq_base', 'rope_freq_scale', 'yarn_ext_factor',
    'yarn_attn_factor', 'yarn_beta_slow', 'yarn_beta_fast', 'slot_prompt_similarity',
  ]
  const search = page.getByRole('textbox', { name: '参数搜索' })
  for (const field of fields) {
    await search.fill(`--${field.replace(/_/g, '-')}`)
    const input = page.locator(`[data-config-field="${field}"] input`)
    await input.fill('0.51')
    expect(await input.evaluate(element => (element as HTMLInputElement).checkValidity()), field).toBe(true)
  }
  await search.fill('--temp')
  const temp = page.locator('[data-config-field="temp"] input')
  for (const value of ['0.123456789', '1e-8']) {
    await temp.fill(value)
    expect(await temp.evaluate(element => (element as HTMLInputElement).validity.valid)).toBe(true)
  }
  await temp.fill('2.1')
  expect(await temp.evaluate(element => (element as HTMLInputElement).validity.rangeOverflow)).toBe(true)
})

test('integer controls accept arbitrary integers and retain incomplete intent edits', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters')
  const search = page.getByRole('textbox', { name: '参数搜索' })
  for (const [field, value] of [['reasoning_budget', '256'], ['cache_ram', '8192'], ['moe_cache_mib', '513']]) {
    await search.fill(`--${field.replace(/_/g, '-')}`)
    const input = page.locator(`[data-config-field="${field}"] input`)
    await input.fill(value)
    expect(await input.evaluate(element => (element as HTMLInputElement).checkValidity()), field).toBe(true)
    await input.fill('256.5')
    expect(await input.evaluate(element => (element as HTMLInputElement).validity.stepMismatch), field).toBe(true)
    await input.press('Tab')
    await expect(input).toHaveValue(value)
  }
  await search.fill('--ctx-size')
  const context = page.locator('[data-config-field="ctx_size"]')
  await context.getByRole('combobox').selectOption('manual')
  const input = context.locator('input')
  await input.fill('2049')
  expect(await input.evaluate(element => (element as HTMLInputElement).checkValidity())).toBe(true)
  await input.fill('')
  await input.press('Tab')
  await expect(input).toHaveValue('2049')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'].ctx_size)).toBe(2049)
})

test('initial load and document reload use the current host snapshot', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=stale-window-snapshot')
  for (let iteration = 0; iteration < 2; iteration += 1) {
    await expect(page.getByText('External CLI instance', { exact: true }).first()).toBeVisible()
    await page.getByText('External CLI instance', { exact: true }).first().click()
    await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
    await expect(page.locator('[data-config-field="temp"] input')).toHaveValue('0.51')
    if (iteration === 0) {
      await page.evaluate(() => localStorage.setItem('lastTab', 'instances'))
      await page.reload()
    }
  }
})

test('numeric parameter edits accept negative sentinel values', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters')
  await page.getByRole('textbox', { name: '参数搜索' }).fill('--adaptive-target')
  const target = page.locator('[data-config-field="adaptive_target"] input')
  await target.fill('')
  await target.pressSequentially('-1')
  await expect(target).toHaveValue('-1')
})

test('decimal values survive mode switches, save, navigation and restoring inheritance', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters')
  const search = page.getByRole('textbox', { name: '参数搜索' })
  const temp = page.locator('[data-config-field="temp"]')
  await temp.locator('input').fill('0.12345679')
  for (const [field, flag, value] of [
    ['top_p', '--top-p', '0.95'], ['mirostat_lr', '--mirostat-lr', '0.1'],
    ['xtc_threshold', '--xtc-threshold', '0.1'], ['adaptive_decay', '--adaptive-decay', '0.9'],
  ]) {
    await search.fill(flag)
    const input = page.locator(`[data-config-field="${field}"] input`)
    await input.fill('0.2')
    await input.fill(value)
  }
  await page.getByRole('button', { name: /手动启动命令/ }).click()
  await page.getByRole('textbox', { name: '完整启动命令或可执行文件后的参数' }).fill('-m model.gguf --temp 0.7')
  await page.getByRole('button', { name: /托管参数模式/ }).click()
  await search.fill('--temp')
  await expect(temp.locator('input')).toHaveValue('0.12345679')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  const saved = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'])
  expect(saved).toMatchObject({ temp: 0.12345679, top_p: 0.95, mirostat_lr: 0.1, xtc_threshold: 0.1, adaptive_decay: 0.9, manual_command: '-m model.gguf --temp 0.7' })
  expect(saved.explicit_overrides).toEqual(expect.arrayContaining(['temp', 'top_p', 'mirostat_lr', 'xtc_threshold', 'adaptive_decay']))
  await page.locator('[data-nav-id="instances"]').click()
  await page.locator('[data-nav-id="config"]').click()
  await expect(temp.locator('input')).toHaveValue('0.12345679')
  await expect(temp.locator('[data-config-status="changed"]')).toHaveCount(0)
  await temp.getByRole('button', { name: /参数说明/ }).click()
  await page.getByRole('button', { name: '恢复引擎默认', exact: true }).click()
  await expect(temp.locator('input')).toHaveValue('0.8')
  await expect(temp).toHaveAttribute('data-config-source', 'inherited')
})

test('incomplete numeric edits do not replace a valid value with zero', async ({ page }) => {
  await openConfiguration(page, 'v070-parameters')
  const temp = page.locator('[data-config-field="temp"] input')
  await temp.fill('0.82')
  await temp.fill('')
  await expect(temp).toHaveValue('')
  await temp.press('Tab')
  await expect(temp).toHaveValue('0.82')
  await temp.fill('')
  await temp.pressSequentially('1e-8')
  await temp.press('Tab')
  await expect(temp).toHaveValue('1e-8')
})

test('saving a normalized decimal stays clean and accepts subsequent MCP changes', async ({ page }) => {
  await openConfiguration(page, 'parameter-float-normalization')
  const temp = page.locator('[data-config-field="temp"]')
  await temp.locator('input').fill('0.123456789')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  await expect(temp.locator('input')).toHaveValue('0.12345679')
  await expect(temp.locator('[data-config-status="changed"]')).toHaveCount(0)
  await page.evaluate(async () => {
    const backend = window.__TAURI_BROWSER_TEST__
    backend.state.instances['browser-test-instance'].temp = 0.44
    backend.state.management_revision = 1
    await backend.emitEvent('management-config-changed', { operation: 'instances.update', revision: 1 })
  })
  await expect(temp.locator('input')).toHaveValue('0.44')
})

for (const english of [false, true]) {
  test(`v0.7.0 parameter help and layout in ${english ? 'English' : 'Chinese'}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: english ? 1440 : 1024, height: 900 })
    await openConfiguration(page, 'v070-parameters')
    if (english) await page.getByRole('button', { name: 'EN', exact: true }).click()
    const search = page.getByRole('textbox', { name: english ? 'Parameter Search' : '参数搜索' })
    await search.fill('--spec-draft')
    const sampling = page.locator('[data-config-field="spec_draft_sampling"]')
    await expect(sampling).toContainText(english ? 'Draft Sampling' : '草稿采样方式')
    await sampling.getByRole('button', { name: english ? /Parameter help/ : /参数说明/ }).click()
    await expect(page.getByRole('tooltip')).toContainText('greedy')
    await page.keyboard.press('Escape')
    await sampling.scrollIntoViewIfNeeded()
    await page.screenshot({ path: testInfo.outputPath('v070-parameters.png') })
    const overflow = await sampling.evaluate(element => element.scrollWidth > element.clientWidth + 1)
    expect(overflow).toBe(false)
    await search.fill('--cache-ram')
    await page.locator('[data-config-field="cache_ram"]').getByRole('button', { name: english ? /Parameter help/ : /参数说明/ }).click()
    await expect(page.getByRole('tooltip')).toContainText('decision')
    await expect(page.getByRole('tooltip')).toContainText('8192')
  })
}

test('opening an instance config keeps React hook order stable (issue #5)', async ({ page }) => {
  const pageErrors: string[] = []
  page.on('pageerror', error => pageErrors.push(error.message))

  await openConfiguration(page)
  await expect(page.locator('[data-config-field="temp"]')).toBeVisible()
  expect(pageErrors).toEqual([])
})

for (const oldEngine of [false, true]) {
  test(`backend sampling dependency follows the ${oldEngine ? 'old' : 'current'} engine build`, async ({ page }) => {
    await openConfiguration(page, `parameter-compatibility${oldEngine ? '-old' : ''}`)
    await page.getByRole('textbox', { name: '参数搜索' }).fill('--backend-sampling')
    const backendSampling = page.locator('[data-config-field="backend_sampling"]')
    await expect(backendSampling).toHaveAttribute('data-config-source', oldEngine ? 'inactive' : 'explicit')
    await expect(page.getByText(/此引擎早于官方 b10355/)).toHaveCount(oldEngine ? 1 : 0)
    await page.getByRole('textbox', { name: '参数搜索' }).fill('--fit')
    for (const key of ['fit_target', 'fit_ctx']) {
      await expect(page.locator(`[data-config-field="${key}"]`)).toHaveAttribute('data-config-source', 'explicit')
    }
    await expect(page.locator('[data-config-field="fit_mode"]')).toHaveAttribute('data-config-source', 'inherited')
  })
}

test('unsaved configuration survives page navigation and saves against the original baseline', async ({ page }) => {
  await openConfiguration(page)
  const temperature = page.locator('[data-config-field="temp"]')
  await temperature.locator('input').fill('0.91')
  await page.locator('[data-nav-id="instances"]').click()
  await page.locator('[data-nav-id="config"]').click()
  await expect(temperature.locator('input')).toHaveValue('0.91')
  await expect(temperature.locator('[data-config-status="changed"]')).toBeVisible()
  expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.temp)).toBe(0.6)
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  await page.locator('[data-nav-id="instances"]').click()
  await page.locator('[data-nav-id="config"]').click()
  await expect(temperature.locator('input')).toHaveValue('0.91')
  await expect(temperature.locator('[data-config-status="changed"]')).toHaveCount(0)
})

test('external management preserves a dirty parameter draft and refuses its stale save', async ({ page }) => {
  await openConfiguration(page)
  const temperature = page.locator('[data-config-field="temp"] input')
  await temperature.fill('0.91')
  await page.evaluate(async () => {
    const backend = window.__TAURI_BROWSER_TEST__
    backend.state.instances['browser-test-instance'].temp = 0.42
    backend.state.management_revision = 1
    await backend.emitEvent('management-config-changed', { operation: 'instances.update', revision: 1 })
  })
  await expect(temperature).toHaveValue('0.91')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'save_config').length)).toBe(1)
  expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'].temp)).toBe(0.42)
  expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.saveCount)).toBe(0)
  await expect(temperature).toHaveValue('0.91')
})

test('external management refreshes a clean parameter page with its new revision', async ({ page }) => {
  await openConfiguration(page)
  await page.evaluate(async () => {
    const backend = window.__TAURI_BROWSER_TEST__
    backend.state.instances['browser-test-instance'].temp = 0.42
    backend.state.management_revision = 1
    await backend.emitEvent('management-config-changed', { operation: 'instances.update', revision: 1 })
  })
  const temperature = page.locator('[data-config-field="temp"] input')
  await expect(temperature).toHaveValue('0.42')
  await temperature.fill('0.72')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
  const save = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.find(call => call.command === 'save_config'))
  expect(save?.payload).toMatchObject({ expectedManagementRevision: 1 })
})

test('switching instances keeps their configuration drafts independent', async ({ page }) => {
  await openConfiguration(page, 'monitoring')
  const temperature = page.locator('[data-config-field="temp"] input')
  await temperature.fill('0.91')
  await page.locator('[data-nav-id="instances"]').click()
  await page.getByText('Stopped Monitoring Instance', { exact: true }).first().click()
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  await expect(temperature).toHaveValue('0.6')
  await temperature.fill('0.72')
  await page.locator('[data-nav-id="instances"]').click()
  await page.getByRole('button', { name: '配置参数', exact: true }).first().click()
  await expect(temperature).toHaveValue('0.91')
})

test('saving a retained draft preserves settings changed on another page', async ({ page }) => {
  await openConfiguration(page)
  const temperature = page.locator('[data-config-field="temp"] input')
  await temperature.fill('0.91')
  await page.locator('[data-nav-id="instances"]').click()
  await page.getByRole('main').getByRole('switch').first().click()
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.auto_start)).toBe(true)
  await page.locator('[data-nav-id="config"]').click()
  await expect(temperature).toHaveValue('0.91')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '2')
  const persisted = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'])
  expect(persisted?.temp).toBe(0.91)
  expect(persisted?.auto_start).toBe(true)
})

for (const fail of [false, true]) {
  test(`navigation during a ${fail ? 'failed' : 'successful'} save preserves newer edits`, async ({ page }) => {
    await openConfiguration(page, 'delayed-config-save')
    const temperature = page.locator('[data-config-field="temp"]')
    await temperature.locator('input').fill('0.7')
    await page.getByRole('button', { name: '保存配置', exact: true }).click()
    await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'save_config').length)).toBe(1)
    await page.locator('[data-nav-id="instances"]').click()
    await page.locator('[data-nav-id="config"]').click()
    await expect(page.getByRole('button', { name: '正在写入...', exact: true })).toBeDisabled()
    await temperature.locator('input').fill('0.8')
    await page.evaluate(shouldFail => window.__TAURI_BROWSER_TEST__.releaseSave(shouldFail), fail)
    if (!fail) await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
    await expect(page.getByRole('button', { name: '保存配置', exact: true })).toBeEnabled()
    await expect(temperature.locator('input')).toHaveValue('0.8')
    await expect(temperature.locator('[data-config-status="changed"]')).toBeVisible()
    expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.temp)).toBe(fail ? 0.6 : 0.7)
  })
}

test('checkpoint requirements are explicit and repaired only after confirmation', async ({ page }) => {
  await openConfiguration(page, 'checkpoint-requirements')

  await expect(page.getByText('当前构建、模型或配置尚未验证多 slot，请使用 1 个 slot')).toBeVisible()
  await expect(page.getByText('必须启用 prompt cache')).toBeVisible()
  await expect(page.getByText(/Harness 元数据请求之后保留已恢复前缀/)).toBeVisible()
  await expect(page.getByText(/滑动窗口模型必须启用 SWA 完整缓存/)).toBeVisible()
  await expect(page.getByText('需要启用上下文检查点（ctx-checkpoints 大于 0）')).toBeVisible()

  await page.getByRole('button', { name: '应用必需设置', exact: true }).click()
  await expect(page.getByText('可使用受管检查点', { exact: true })).toBeVisible()

  const repaired = await page.evaluate(() => {
    const calls = window.__TAURI_BROWSER_TEST__.calls
      .filter(candidate => candidate.command === 'get_checkpoint_eligibility')
    const call = calls[calls.length - 1]
    return (call?.payload as { config?: Record<string, unknown> } | undefined)?.config
  })
  expect(repaired).toMatchObject({
    parallel: 1,
    cache_prompt: true,
    cache_idle_slots: true,
    cache_ram: 8192,
    slots_enabled: true,
    swa_full: true,
    ctx_checkpoints: 32,
  })
})

test('repairing reviewed multi-slot retention preserves its qualified topology', async ({ page }) => {
  await openConfiguration(page, 'checkpoint-multi-slot')
  const repair = page.getByRole('button', { name: '应用必需设置', exact: true })
  await repair.click()
  await expect(page.getByText('可使用受管检查点', { exact: true })).toBeVisible()
  await expect(repair).toHaveCount(0)
  const config = await page.evaluate(() => {
    const calls = window.__TAURI_BROWSER_TEST__.calls.filter(c => c.command === 'get_checkpoint_eligibility')
    return (calls[calls.length - 1]?.payload as { config?: Record<string, unknown> })?.config
  })
  expect(config).toMatchObject({ parallel: 4, cache_idle_slots: false, cache_ram: 256 })
})

test('idle checkpoint and longer-prefix retention are explicit saved policies', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 900 })
  await openConfiguration(page, 'checkpoint-requirements')
  const panel = page.locator('#config-checkpoint')
  await panel.scrollIntoViewIfNeeded()
  const idle = panel.getByRole('spinbutton', { name: /空闲后保存/ })
  await expect(idle).toHaveValue('0')
  await idle.fill('5')
  await panel.getByRole('checkbox', { name: '保留较长检查点', exact: true }).click()
  await panel.getByRole('checkbox', { name: '受控停止时保存', exact: true }).click()
  await expect(idle).toBeDisabled()
  await panel.getByRole('checkbox', { name: '受控停止时保存', exact: true }).click()
  await expect(idle).toBeEnabled()
  await expect(idle).toHaveValue('5')
  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'].kv_checkpoint)).toMatchObject({
    idle_save_seconds: 5, preserve_longer: true, auto_save: true,
  })
  await panel.screenshot({ path: 'test-results/checkpoint-idle-zh-1024.png' })
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

  const lazyMode = page.locator('[data-config-field="lazy_mode"]')
  const lazyModeSelect = lazyMode.getByRole('combobox')
  await expect(lazyModeSelect).toHaveValue('')
  await lazyModeSelect.selectOption('on')
  await expect(lazyMode).toHaveAttribute('data-config-source', 'explicit')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .toEqual(expect.arrayContaining(['--lazy-mode', 'on']))
})

test('speculative decoding accepts a normalized multi-type fallback chain', async ({ page }) => {
  await openConfiguration(page)

  const reasoning = page.locator('[data-config-field="reasoning"]')
  const specType = page.locator('[data-config-field="spec_type"]')
  const draftTokens = page.locator('[data-config-field="draft_tokens"]')
  const trigger = specType.getByRole('button', { name: '推测解码类型 (--spec-type)', exact: true })
  const beforeReasoningHeight = await reasoning.evaluate(element => element.getBoundingClientRect().height)
  const beforeSpecTypeHeight = await specType.evaluate(element => element.getBoundingClientRect().height)
  const beforeDraftTokensTop = await draftTokens.evaluate(element => (element as HTMLElement).offsetTop)

  await expect(specType.locator('[data-spec-type-source]')).toHaveAttribute('data-spec-type-source', 'engine')
  await trigger.click()

  const popup = page.getByRole('listbox', { name: '推测解码类型 (--spec-type)', exact: true })
  await expect(popup).toBeVisible()
  await expect(popup).toHaveCSS('position', 'fixed')
  expect(await popup.evaluate(element => element.parentElement === document.body)).toBe(true)

  const afterReasoningHeight = await reasoning.evaluate(element => element.getBoundingClientRect().height)
  const afterSpecTypeHeight = await specType.evaluate(element => element.getBoundingClientRect().height)
  const afterDraftTokensTop = await draftTokens.evaluate(element => (element as HTMLElement).offsetTop)
  expect(Math.abs(afterReasoningHeight - beforeReasoningHeight)).toBeLessThanOrEqual(1)
  expect(Math.abs(afterSpecTypeHeight - beforeSpecTypeHeight)).toBeLessThanOrEqual(1)
  expect(Math.abs(afterDraftTokensTop - beforeDraftTokensTop)).toBeLessThanOrEqual(1)

  await popup.getByRole('checkbox', { name: 'draft-mtp', exact: true }).check()
  await popup.getByRole('checkbox', { name: 'ngram-mod', exact: true }).check()

  await expect(trigger).toContainText('ngram-mod,draft-mtp')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .toEqual(expect.arrayContaining(['--spec-type', 'ngram-mod,draft-mtp']))

  await page.keyboard.press('Escape')
  await expect(popup).toBeHidden()
  await expect(trigger).toBeFocused()
  await trigger.click()
  await expect(popup).toBeVisible()
  await page.locator('[data-config-field="temp"] input').click()
  await expect(popup).toBeHidden()

  await page.getByRole('button', { name: '保存配置', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.spec_type
  ))).toBe('ngram-mod,draft-mtp')

  await page.getByRole('button', { name: '实例管理', exact: true }).click()
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  const reopenedSpecType = page.locator('[data-config-field="spec_type"]')
  const reopenedTrigger = reopenedSpecType.getByRole('button', { name: '推测解码类型 (--spec-type)', exact: true })
  await expect(reopenedTrigger).toContainText('ngram-mod,draft-mtp')
  await reopenedTrigger.click()
  await page.getByRole('listbox', { name: '推测解码类型 (--spec-type)', exact: true })
    .getByRole('button', { name: 'none', exact: true }).click()
  await expect(reopenedTrigger).toContainText('none')
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.lastGenerated?.command ?? []))
    .not.toContain('--spec-type')
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
