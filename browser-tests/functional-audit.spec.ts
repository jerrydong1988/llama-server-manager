import { expect, test, type Page } from '@playwright/test'

async function open(page: Page, tab: string) {
  await page.addInitScript(tab => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', tab)
  }, tab)
  await page.goto('/?scenario=proxy-routing')
}

test('router lifecycle refreshes CAS revision before a subsequent save', async ({ page }) => {
  await open(page, 'proxy')
  await page.getByRole('button', { name: '停止', exact: true }).click()
  await expect(page.getByRole('button', { name: '启动', exact: true })).toBeEnabled()
  await page.getByRole('combobox', { name: '调度策略' }).selectOption('weighted')
  await page.getByTestId('proxy-header-save').click()
  await expect.poll(() => page.evaluate(async () => {
    const result = await (window as any).__TAURI_INTERNALS__.invoke('get_proxy_config')
    return result.routing_strategy
  })).toBe('weighted')
  await expect(page.getByTestId('proxy-header-save')).toBeEnabled()
  await page.getByRole('button', { name: '启动', exact: true }).click()
  await expect(page.getByRole('button', { name: '停止', exact: true })).toBeEnabled()
  await page.getByRole('combobox', { name: '调度策略' }).selectOption('priorityFailover')
  await page.getByTestId('proxy-header-save').click()
  await expect.poll(() => page.evaluate(async () => (await (window as any).__TAURI_INTERNALS__.invoke('get_proxy_config')).routing_strategy)).toBe('priorityFailover')
})

test('late router save receipt retains edits entered while saving', async ({ page }) => {
  await open(page, 'proxy')
  await expect(page.getByTestId('proxy-header-save')).toBeVisible()
  await page.evaluate(() => {
    const api = (window as any).__TAURI_INTERNALS__
    const original = api.invoke
    api.invoke = async (command: string, ...args: unknown[]) => {
      if (command === 'save_proxy_config') await new Promise<void>(resolve => { (window as any).releaseAuditSave = resolve })
      return original(command, ...args)
    }
  })
  const strategy = page.getByRole('combobox', { name: '调度策略' })
  await strategy.selectOption('weighted')
  await page.getByTestId('proxy-header-save').click()
  await page.waitForFunction(() => Boolean((window as any).releaseAuditSave))
  await strategy.selectOption('priorityFailover')
  await page.evaluate(() => (window as any).releaseAuditSave())
  await expect(page.getByTestId('proxy-header-save')).toBeEnabled()
  await expect(strategy).toHaveValue('priorityFailover')
  expect(await page.evaluate(async () => (await (window as any).__TAURI_INTERNALS__.invoke('get_proxy_config')).routing_strategy)).toBe('weighted')
})

for (const failure of ['reference', 'write', 'revision', 'none']) {
  test(`instance deletion is atomic on ${failure}`, async ({ page }) => {
    await open(page, 'instances')
    await expect(page.locator('.instance-list-row').first()).toBeVisible()
    const name = await page.evaluate(async failure => {
      const api = (window as any).__TAURI_INTERNALS__
      const original = api.invoke
      const config = await original('get_proxy_config')
      config.default_instance_id = ''
      config.routes = failure === 'reference'
        ? [{ id: 'audit-reference', enabled: true, model_alias: 'audit', target_instance_id: 'browser-test-instance', priority: 0, weight: 1 }]
        : []
      await original('save_proxy_config', { config })
      ;(window as any).auditDeleteCalls = []
      api.invoke = async (command: string, ...args: unknown[]) => {
        ;(window as any).auditDeleteCalls.push(command)
        if (command.startsWith('plugin:dialog|')) return 'Ok'
        if (command === 'delete_instance_config' && failure === 'write') throw new Error('disk write failed')
        if (command === 'delete_instance_config' && failure === 'revision') throw new Error('CONFIG_CONFLICT')
        return original(command, ...args)
      }
      return window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'].name
    }, failure)
    const row = page.locator('.instance-list-row').filter({ hasText: name }).first()
    await row.locator('summary').click()
    await row.getByRole('button', { name: '删除实例', exact: true }).click()
    await expect.poll(() => page.evaluate(() => (window as any).auditDeleteCalls.includes('delete_instance_config'))).toBe(true)
    if (failure !== 'none') {
      await expect(row).toBeVisible()
      expect(await page.evaluate(() => Boolean(window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']))).toBe(true)
      expect(await page.evaluate(() => (window as any).auditDeleteCalls.filter((c: string) => ['clear_checkpoint', 'delete_instance_log'].includes(c)))).toEqual([])
    } else {
      await expect(row).toHaveCount(0)
      await expect.poll(() => page.evaluate(() => (window as any).auditDeleteCalls.includes('delete_instance_log'))).toBe(true)
      const calls = await page.evaluate(() => (window as any).auditDeleteCalls as string[])
      expect(calls.indexOf('delete_instance_config')).toBeLessThan(calls.indexOf('clear_checkpoint'))
      expect(calls.indexOf('clear_checkpoint')).toBeLessThan(calls.indexOf('delete_instance_log'))
    }
  })
}
