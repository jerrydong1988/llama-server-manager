import { expect, test, type Page } from '@playwright/test'

// Check painted content, not intrinsic select/input text widths. Scrollable
// tables and intentionally truncated labels are allowed to retain their text.
async function expectContainedLayout(page: Page) {
  await expect.poll(() => page.evaluate(() => {
    const issues: string[] = []
    for (const element of document.querySelectorAll<HTMLElement>('main div, main section, main aside, main p, main h2, main h3, main label, main button, main input, main select, main textarea')) {
      const box = element.getBoundingClientRect()
      const style = getComputedStyle(element)
      if (!box.width || !box.height || style.visibility === 'hidden') continue
      const control = element.matches('input,select,textarea')
      if (!control && style.overflowX === 'visible' && element.scrollWidth > element.clientWidth + 2) {
        issues.push(`${element.tagName}: ${element.textContent?.slice(0, 70)}`)
      }
      if (control && element.parentElement) {
        const parent = element.parentElement.getBoundingClientRect()
        if (box.left < parent.left - 2 || box.right > parent.right + 2) issues.push(`control: ${element.getAttribute('aria-label') || element.tagName}`)
      }
    }
    return issues
  })).toEqual([])
}

async function openStressLayout(page: Page, language: string) {
  await page.addInitScript(lang => { localStorage.setItem('lang', lang); localStorage.setItem('lastTab', 'instances') }, language)
  await page.goto('/?scenario=docs-screenshots&layout=stress')
  await expect(page.getByRole('button', { name: 'B11046 (ROCm)', exact: true }).first()).toBeVisible()
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

for (const language of ['zh-CN', 'en-US']) {
  const zh = language === 'zh-CN'
  for (const width of [1024, 1280, 1920]) {
    test(`application panels contain long names at ${width}px in ${language}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 })
      await openStressLayout(page, language)
      const tabs = await page.locator('[data-nav-id]').evaluateAll(items => items.map(item => item.getAttribute('data-nav-id')!))
      for (const tab of tabs) {
        await page.locator(`[data-nav-id="${tab}"]`).click()
        await page.waitForLoadState('networkidle')
        await expectContainedLayout(page)
        await page.getByRole('button', { name: zh ? '切换到明亮模式' : 'Switch to light mode', exact: true }).click()
        await expectContainedLayout(page)
        await page.getByRole('button', { name: zh ? '切换到深色模式' : 'Switch to dark mode', exact: true }).click()
      }
      await page.locator('[data-nav-id="proxy"]').click()
      const table = page.getByRole('table')
      const widths = await table.evaluate(element => ({ table: element.getBoundingClientRect().width, wrapper: element.parentElement!.clientWidth }))
      expect(widths.table).toBeGreaterThanOrEqual(1242)
      if (width === 1024) expect(widths.table).toBeGreaterThan(widths.wrapper)
      const action = table.getByRole('button', { name: zh ? '测试路由' : 'Test Route', exact: true }).first()
      await action.scrollIntoViewIfNeeded()
      await expect(action).toBeInViewport()
    })
  }

  test(`instance dialogs and expanded configuration contain long content in ${language}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1024, height: 900 })
    await openStressLayout(page, language)
    await page.locator('[data-guide="instance-create"]').click()
    const modal = page.locator('.fixed.inset-0.z-50')
    const engine = modal.locator('select')
    for (const value of ['browser-test-engine', 'browser-vulkan-engine', '']) {
      await engine.selectOption(value)
      const selectBox = await engine.boundingBox()
      const portBox = await modal.locator('input[type="number"]').boundingBox()
      expect(Math.abs(selectBox!.width - portBox!.width)).toBeLessThan(1)
      await expectContainedLayout(page)
    }
    await engine.selectOption('browser-test-engine')
    await modal.screenshot({ path: testInfo.outputPath('create-instance.png') })
    await modal.getByRole('button', { name: 'Close', exact: true }).click()
    await page.getByRole('button', { name: 'B11046 (ROCm)', exact: true }).first().click()
    await expectContainedLayout(page)
    await expect(modal.getByRole('button', { name: 'Close', exact: true })).toBeInViewport()
    await modal.getByRole('button', { name: 'Close', exact: true }).click()
    await page.getByRole('button', { name: zh ? '配置参数' : 'Configure', exact: true }).last().click()
    await page.getByRole('textbox', { name: zh ? '参数搜索' : 'Parameter Search', exact: true }).fill('--')
    await expect(page.locator('[data-config-field="rpc_servers"]')).toBeVisible()
    await expectContainedLayout(page)
    await page.locator('[data-nav-id="proxy"]').click()
    await page.getByRole('tab', { name: zh ? '使用统计' : 'Usage statistics', exact: true }).click()
    await expect(page.getByTestId('router-usage-panel')).toBeVisible()
    await expectContainedLayout(page)
  })

  test(`worker launch previews wrap long paths in ${language}`, async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 900 })
    await openStressLayout(page, language)
    await page.locator('[data-nav-id="cluster"]').click()
    await page.getByRole('button', { name: zh ? '本地启动' : 'Local Launch', exact: true }).click()
    const dialog = page.getByRole('dialog')
    await expectContainedLayout(page)
    await dialog.locator('input[type="radio"]').last().check()
    await dialog.locator('input[type="text"]').fill(`C:\\Models\\${'LongPath'.repeat(35)}`)
    await expectContainedLayout(page)
    await dialog.getByRole('button', { name: zh ? '取消' : 'Cancel', exact: true }).first().click()
    await page.getByRole('button', { name: zh ? '一键启动 Worker' : 'One-Click Launch Worker', exact: true }).click()
    await dialog.locator('input[type="text"]').last().fill(`C:\\Models\\${'LongPath'.repeat(35)}\\rpc-server.exe`)
    await dialog.getByRole('button', { name: zh ? '下一步' : 'Next', exact: true }).click()
    await dialog.getByRole('button', { name: zh ? '下一步' : 'Next', exact: true }).click()
    await expectContainedLayout(page)
  })
}
