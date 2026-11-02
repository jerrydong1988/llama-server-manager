import { expect, test } from '@playwright/test'

test('a failed Gufo save does not roll back a newer instance update', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/?scenario=gufo-delayed-config-save')
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  await page.getByLabel('启动等待上限（秒）').fill('900')
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
  await page.getByLabel('推测解码').selectOption('dflash2')
  await page.getByLabel('匹配的 DFlash2 草稿').fill('C:\\models\\draft.gguf')
  await page.getByRole('combobox', { name: '内存缓存', exact: true }).selectOption('off')
  await page.getByLabel('启用磁盘缓存').check()
  await page.getByLabel('启动等待上限（秒）').fill('900')
  await page.getByRole('button', { name: '校验并保存' }).click()
  await expect(page.getByRole('status')).toContainText('配置已保存')
  const saved = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'])
  expect(saved.engine_kind).toBe('gufo')
  expect(saved.gufo).toMatchObject({ model_family: 'qwen38_27b', speculative: 'dflash2', cache_ram_mode: 'off', cache_disk: true, startup_timeout_seconds: 900 })
  await page.setViewportSize({ width: 900, height: 900 })
  await expect(page.getByLabel('主模型 GGUF（第一分片）')).toBeVisible()
  await page.screenshot({ path: 'test-results/gufo-config.png', fullPage: true })
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})
