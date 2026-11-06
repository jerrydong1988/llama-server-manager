import { expect, test } from '@playwright/test'

test('a failed Gufo save does not roll back a newer instance update', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=gufo-delayed-config-save')
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  await page.getByLabel('启动等待上限（秒）').fill('900')
  await expect(page.getByLabel('思考预算（Token 数）', { exact: true })).toHaveValue('-1')
  await page.getByLabel('思考预算（Token 数）', { exact: true }).fill('4096')
  await page.getByRole('button', { name: '校验并保存' }).click()
  await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'save_config').length)).toBe(1)
  await page.evaluate(async () => {
    const modulePath = '/src/store.ts'
    const { useAppStore } = await import(/* @vite-ignore */ modulePath)
    const store = useAppStore.getState()
    const instance = store.instances.find((item: { id: string }) => item.id === 'browser-test-instance')
    store.updateInstance(instance.id, { config: { ...instance.config, alias: 'newer-alias' } })
    window.__TAURI_BROWSER_TEST__.releaseSave(true)
  })
  await expect(page.getByRole('alert')).toHaveText('Simulated configuration write failure')
  expect(await page.evaluate(async () => {
    const modulePath = '/src/store.ts'
    const { useAppStore } = await import(/* @vite-ignore */ modulePath)
    return useAppStore.getState().instances.find((item: { id: string }) => item.id === 'browser-test-instance').config.alias
  })).toBe('newer-alias')
  await expect(page.getByLabel('API 模型名称')).toHaveValue('newer-alias')
  await expect(page.getByLabel('启动等待上限（秒）')).toHaveValue('900')
  await expect(page.getByLabel('思考预算（Token 数）', { exact: true })).toHaveValue('4096')
})

test('Gufo config exposes supported fields, persists the draft, and separates cache policy', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=gufo')
  await expect(page.getByRole('button', { name: '配置参数', exact: true }).last()).toBeVisible()
  await expect(page.getByRole('button', { name: '清除检查点', exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  await expect(page.getByRole('heading', { name: /Gufo 参数配置/ })).toBeVisible()
  await expect(page.getByRole('textbox', { name: '参数搜索' })).toHaveCount(0)
  await page.getByLabel('模型家族').selectOption('qwen38_27b')
  await page.getByRole('combobox', { name: '推测解码', exact: true }).selectOption('dflash2')
  await page.getByLabel('匹配的 DFlash2 草稿', { exact: true }).fill('C:\\models\\draft.gguf')
  await page.getByRole('combobox', { name: '内存缓存', exact: true }).selectOption('off')
  await page.getByLabel('启用磁盘缓存').check()
  await page.getByLabel('启动等待上限（秒）').fill('900')
  await page.getByLabel('思考预算（Token 数）', { exact: true }).fill('4096')
  await page.getByRole('button', { name: '校验并保存' }).click()
  await expect(page.getByRole('status')).toContainText('配置已保存')
  const saved = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'])
  expect(saved.engine_kind).toBe('gufo')
  expect(saved.gufo).toMatchObject({ model_family: 'qwen38_27b', speculative: 'dflash2', cache_ram_mode: 'off', cache_disk: true, startup_timeout_seconds: 900, reasoning_budget: 4096 })
  const preview = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.find(call => call.command === 'generate_server_command'))
  expect(preview?.payload).toMatchObject({ config: { gufo: { reasoning_budget: 4096 } } })
  await page.getByRole('button', { name: '返回实例管理', exact: true }).click()
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  await expect(page.getByLabel('思考预算（Token 数）', { exact: true })).toHaveValue('4096')
  await page.setViewportSize({ width: 900, height: 900 })
  await expect(page.getByLabel('主模型 GGUF（第一分片）', { exact: true })).toBeVisible()
  await page.screenshot({ path: 'test-results/gufo-config.png', fullPage: true })
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('Gufo reasoning budget rejects invalid input and preserves zero and unlimited', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=gufo')
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  const budget = page.getByLabel('思考预算（Token 数）', { exact: true })
  await expect(budget).toHaveValue('-1')
  for (const value of ['', '-2', '1.5', '9007199254740992']) {
    await budget.fill(value)
    for (const button of ['预览命令', '校验并保存']) {
      await page.getByRole('button', { name: button, exact: true }).click()
      await expect(page.getByRole('alert')).toContainText('思考预算请输入')
    }
  }
  expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.filter(call => ['save_config', 'generate_server_command'].includes(call.command)).length)).toBe(0)
  for (const value of ['0', '-1']) {
    await budget.fill(value)
    await page.getByRole('button', { name: '校验并保存' }).click()
    await expect(page.getByRole('status')).toContainText('配置已保存')
    expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'].gufo.reasoning_budget)).toBe(Number(value))
  }
})

for (const language of ['zh-CN', 'en-US']) {
  test(`Gufo grouped layout contains long paths and conditional controls in ${language}`, async ({ page }, testInfo) => {
    await page.addInitScript(lang => {
      localStorage.setItem('lang', lang)
      localStorage.setItem('lastTab', 'instances')
    }, language)
    await page.goto('/?scenario=gufo')
    await page.evaluate(async () => {
      const modulePath = '/src/store.ts'
      const { useAppStore } = await import(/* @vite-ignore */ modulePath)
      const store = useAppStore.getState()
      const instance = store.instances.find((item: { id: string }) => item.id === 'browser-test-instance')
      const name = 'Gufo · Qwen3.8-Flash-Next · MTP'
      store.updateInstance(instance.id, { name, config: { ...instance.config, name } })
    })
    await page.getByRole('button', { name: language === 'zh-CN' ? '配置参数' : 'Configure', exact: true }).last().click()
    const form = page.getByTestId('gufo-config-page')
    const longPath = 'C:\\模型目录 with spaces\\' + 'Qwen3.8-Flash-Next-long-directory\\'.repeat(5) + 'Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf'
    const model = form.locator('input[id$="-model_path"]')
    await model.fill(longPath)
    await expect(model).toHaveAttribute('title', longPath)
    await model.focus()
    await page.keyboard.press('Tab')
    await expect(form.getByRole('button', { name: language === 'zh-CN' ? '选择文件: 主模型 GGUF（第一分片）' : 'Select file: Model GGUF (first shard)', exact: true })).toBeFocused()
    await page.keyboard.press('Tab')
    await expect(form.locator('input[id$="-mmproj_path"]')).toBeFocused()
    await form.locator('select[id$="-speculative"]').selectOption('mtp')
    await form.locator('input[id$="-mtp_model"]').fill(longPath)
    await form.locator('select[id$="-cache_ram_mode"]').selectOption('limit')
    await form.getByRole('checkbox', { name: language === 'zh-CN' ? '启用磁盘缓存' : 'Enable disk cache' }).check()
    await form.locator('summary').click()
    for (const width of [560, 1024, 1920]) {
      await page.setViewportSize({ width, height: width === 1920 ? 1800 : 1000 })
      for (const dark of [true, false]) {
        const themeButton = page.getByRole('button', { name: language === 'zh-CN'
          ? dark ? '切换到深色模式' : '切换到明亮模式'
          : dark ? 'Switch to dark mode' : 'Switch to light mode', exact: true })
        if (await themeButton.count()) await themeButton.click()
        const issues = await form.evaluate(root => Array.from(root.querySelectorAll<HTMLElement>('section, p, label, input, select, summary, button')).flatMap(element => {
          const box = element.getBoundingClientRect()
          if (!box.width || !box.height) return []
          const parent = element.parentElement!.getBoundingClientRect()
          const outsideParent = box.left < parent.left - 2 || box.right > parent.right + 2
          const style = getComputedStyle(element)
          const overflows = !element.matches('input,select') && style.overflowX === 'visible' && element.scrollWidth > element.clientWidth + 2
          return outsideParent || overflows ? [element.id || element.tagName + ': ' + element.textContent?.slice(0, 50)] : []
        }))
        expect(issues).toEqual([])
        if (width === 1920) {
          await form.locator('summary').click()
          await form.screenshot({ path: testInfo.outputPath(`gufo-layout-${dark ? 'dark' : 'light'}.png`), animations: 'disabled' })
          await form.locator('summary').click()
        }
      }
    }
    await form.locator('select[id$="-model_family"]').selectOption('qwen38_27b')
    await expect(form.locator('input[id$="-mtp_model"]')).toHaveCount(0)
    await expect(form.locator('input[id$="-draft_tokens"]')).toHaveCount(0)
    await form.locator('select[id$="-speculative"]').selectOption('dflash2')
    await expect(form.locator('input[id$="-dflash_model"]')).toBeVisible()
    await form.getByRole('checkbox', { name: language === 'zh-CN' ? '启用磁盘缓存' : 'Enable disk cache' }).uncheck()
    await expect(form.locator('input[id$="-cache_disk_gib"]')).toHaveCount(0)
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
  })
}

test('Gufo file buttons choose the correct asset without replacing other configuration', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('lang', 'zh-CN'); localStorage.setItem('lastTab', 'instances') })
  await page.goto('/?scenario=gufo')
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  const paths = await page.evaluate(async () => {
    const modulePath = '/src/store.ts'
    const { useAppStore } = await import(/* @vite-ignore */ modulePath)
    const base = useAppStore.getState().models[0]
    const root = 'C:\\browser-test\\models\\Gufo 模型'
    const files = [
      { name: 'Qwen-00001-of-00002.gguf', relative: 'Flash\\Qwen-00001-of-00002.gguf', file_type: 'model' },
      { name: 'mmproj-BF16.gguf', relative: 'Flash\\mmproj-BF16.gguf', file_type: 'mmproj' },
      { name: 'mtp-Q8_0.gguf', relative: 'Flash\\MTP\\mtp-Q8_0.gguf', file_type: 'model' },
      { name: 'DFlash2-Q4_K_M.gguf', relative: '27B\\DFlash2-Q4_K_M.gguf', file_type: 'model' },
      { name: 'Qwen-00002-of-00002.gguf', relative: 'Flash\\Qwen-00002-of-00002.gguf', file_type: 'model', is_shard: true },
      { name: 'notes.txt', relative: 'notes.txt', file_type: 'model' },
    ].map((file, index) => ({ ...base, is_shard: false, ...file, id: 'gufo-asset-' + index, path: root + '\\' + file.relative }))
    useAppStore.setState({ models: files, modelDirs: [root] })
    return files.map(file => file.path)
  })
  await page.getByLabel('API 模型名称', { exact: true }).fill('keep-custom-alias')
  await page.getByLabel('思考预算（Token 数）', { exact: true }).fill('8192')
  const picker = page.locator('.fixed.inset-0.z-50').filter({ has: page.getByRole('heading', { name: '从模型仓库选择', exact: true }) })
  await page.getByRole('button', { name: '选择文件: 主模型 GGUF（第一分片）', exact: true }).click()
  await expect(picker.getByRole('button', { name: /Qwen-00002/ })).toHaveCount(0)
  await expect(picker.getByRole('button', { name: /notes.txt/ })).toHaveCount(0)
  await expect(picker.getByRole('button', { name: /mmproj-BF16/ })).toHaveCount(0)
  await picker.getByRole('button', { name: /Qwen-00001/ }).click()
  await expect(page.getByLabel('主模型 GGUF（第一分片）', { exact: true })).toHaveValue(paths[0])
  await page.getByRole('button', { name: '选择文件: 视觉投影 GGUF', exact: true }).click()
  await expect(picker.getByRole('button', { name: /Qwen-00001/ })).toHaveCount(0)
  await picker.getByRole('button', { name: /mmproj-BF16/ }).click()
  await expect(page.getByLabel('视觉投影 GGUF', { exact: true })).toHaveValue(paths[1])
  await page.getByRole('combobox', { name: '推测解码', exact: true }).selectOption('mtp')
  await page.getByRole('button', { name: '选择文件: 匹配的 MTP 模型', exact: true }).click()
  await picker.getByRole('button', { name: 'MTP', exact: true }).click()
  await expect(picker.getByRole('button', { name: /mtp-Q8_0/ })).toHaveCount(0)
  await picker.getByRole('button', { name: 'MTP', exact: true }).click()
  await picker.getByRole('button', { name: /mtp-Q8_0/ }).click()
  await expect(page.getByLabel('匹配的 MTP 模型', { exact: true })).toHaveValue(paths[2])
  await page.getByLabel('模型家族').selectOption('qwen38_27b')
  await page.getByRole('combobox', { name: '推测解码', exact: true }).selectOption('dflash2')
  await page.getByRole('button', { name: '选择文件: 匹配的 DFlash2 草稿', exact: true }).click()
  await picker.getByRole('button', { name: /DFlash2-Q4_K_M/ }).click()
  await expect(page.getByLabel('匹配的 DFlash2 草稿', { exact: true })).toHaveValue(paths[3])
  await page.getByRole('button', { name: '校验并保存', exact: true }).click()
  await expect(page.getByRole('status')).toContainText('配置已保存')
  const saved = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'])
  expect(saved).toMatchObject({ model_path: paths[0], mmproj_path: paths[1], alias: 'keep-custom-alias',
    gufo: { mtp_model: paths[2], dflash_model: paths[3], reasoning_budget: 8192 } })
})

test('Gufo file selection can be cancelled and cannot change a running instance', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('lang', 'zh-CN'); localStorage.setItem('lastTab', 'instances') })
  await page.goto('/?scenario=gufo')
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  const model = page.getByLabel('主模型 GGUF（第一分片）', { exact: true })
  await model.fill('C:\\模型目录 with spaces\\keep.gguf')
  const choose = page.getByRole('button', { name: '选择文件: 主模型 GGUF（第一分片）', exact: true })
  const picker = page.locator('.fixed.inset-0.z-50').filter({ has: page.getByRole('heading', { name: '从模型仓库选择', exact: true }) })
  await choose.click()
  await picker.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(model).toHaveValue('C:\\模型目录 with spaces\\keep.gguf')
  await choose.click()
  await page.evaluate(async () => {
    const modulePath = '/src/store.ts'
    const { useAppStore } = await import(/* @vite-ignore */ modulePath)
    useAppStore.getState().updateInstance('browser-test-instance', { status: 'running' })
  })
  await picker.getByRole('button', { name: /Qwen Browser Test Q8_0/ }).click()
  await expect(picker).toHaveCount(0)
  await expect(model).toHaveValue('C:\\模型目录 with spaces\\keep.gguf')
  await expect(choose).toBeDisabled()
  await expect(page.getByRole('alert')).toContainText('请先停止实例')
  expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'save_config').length)).toBe(0)
  await page.evaluate(async () => {
    const modulePath = '/src/store.ts'
    const { useAppStore } = await import(/* @vite-ignore */ modulePath)
    useAppStore.getState().updateInstance('browser-test-instance', { status: 'stopped' })
    useAppStore.setState({ models: [] })
  })
  await choose.click()
  await expect(picker).toContainText('没有匹配的 GGUF 文件')
  await picker.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(model).toHaveValue('C:\\模型目录 with spaces\\keep.gguf')
})
