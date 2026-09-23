import { expect, test, type Page } from '@playwright/test'

const panel = (page: Page) => page.locator('[data-guide-panel]')
const next = (page: Page) => panel(page).getByRole('button', { name: '下一步', exact: true })

async function openGuide(page: Page, scenario = 'docs-screenshots', lang = 'zh-CN') {
  await page.addInitScript(language => {
    localStorage.setItem('lang', language)
    localStorage.setItem('lastTab', 'guide')
  }, lang)
  await page.goto(`/?scenario=${scenario}`)
  await expect(page.getByRole('button', { name: lang === 'zh-CN' ? '开始交互式引导' : 'Start Interactive Tour' })).toBeVisible()
}

async function startGuide(page: Page) {
  await page.getByRole('button', { name: '开始交互式引导' }).click()
  await expect(panel(page)).toBeVisible()
}

async function reachInstances(page: Page) {
  await startGuide(page)
  await next(page).click()
  await next(page).click()
  await expect(panel(page).getByRole('heading', { name: '创建或选择实例' })).toBeVisible()
}

async function reachVerification(page: Page) {
  await reachInstances(page)
  await panel(page).getByLabel('本次引导的实例').selectOption('browser-test-instance')
  await next(page).click()
  await panel(page).getByRole('button', { name: '已检查配置，继续' }).click()
  await next(page).click()
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('six continuous steps support backward navigation and require a successful connection', async ({ page }) => {
  await openGuide(page)
  await startGuide(page)
  await expect(page.locator('[data-guide-progress]')).toHaveText('第 1 / 6 步')
  await expect(page.locator('[data-guide="model-directories"]')).toHaveClass(/guide-tour-target/)
  await expect(panel(page).getByRole('button', { name: '上一步' })).toBeDisabled()
  await next(page).click()
  await expect(page.locator('[data-guide-progress]')).toHaveText('第 2 / 6 步')
  await panel(page).getByRole('button', { name: '上一步' }).click()
  await expect(page.locator('[data-nav-id="model-repo"]')).toHaveAttribute('aria-current', 'page')
  await next(page).click()
  await next(page).click()
  await panel(page).getByLabel('本次引导的实例').selectOption('browser-test-instance')
  await next(page).click()
  await expect(page.locator('#config-page-actions')).toContainText('Qwen3 8B Chat')
  await panel(page).getByRole('button', { name: '已检查配置，继续' }).click()
  await next(page).click()
  await expect(page.locator('[data-guide-progress]')).toHaveText('第 6 / 6 步')
  await expect(panel(page).getByRole('button', { name: '完成引导' })).toBeDisabled()
  await page.locator('[data-guide="instance-connection"]').click()
  await panel(page).getByRole('button', { name: '完成引导' }).click()
  await expect(panel(page).getByRole('heading', { name: '实例已通过连接测试' })).toBeVisible()
  await panel(page).getByRole('button', { name: '留在当前页面' }).click()
  await expect(panel(page)).toHaveCount(0)
  await expect(page.locator('[data-nav-id="instances"]')).toHaveAttribute('aria-current', 'page')
  await expect(page.locator('.guide-tour-target')).toHaveCount(0)
})

for (const exit of ['escape', 'close', 'during-navigation']) {
  test(`exit ${exit} cancels the entire session without a later page jump`, async ({ page }) => {
    await openGuide(page)
    await startGuide(page)
    if (exit === 'during-navigation') await next(page).click()
    if (exit === 'close') await panel(page).getByRole('button', { name: '退出引导' }).click()
    else await page.keyboard.press('Escape')
    await expect(panel(page)).toHaveCount(0)
    await expect(page.locator('[data-nav-id="guide"]')).toHaveAttribute('aria-current', 'page')
    await page.locator('[data-nav-id="logs"]').click()
    await expect(page.locator('[data-nav-id="logs"]')).toHaveAttribute('aria-current', 'page')
    await expect(page.locator('.guide-tour-target')).toHaveCount(0)
  })
}

test('selected instance is retained and the previous configuration context is restored on exit', async ({ page }) => {
  await openGuide(page)
  await page.locator('[data-nav-id="instances"]').click()
  await page.locator('[role="button"]').filter({ hasText: 'Qwen3 VL 7B' }).click()
  await page.getByRole('button', { name: '配置参数', exact: true }).click()
  await page.locator('[data-nav-id="guide"]').click()
  await reachInstances(page)
  await expect(panel(page).getByLabel('本次引导的实例')).toHaveValue('browser-stopped-instance')
  await next(page).click()
  await expect(page.locator('#config-page-actions')).toContainText('Qwen3 VL 7B')
  await panel(page).getByLabel('本次引导的实例').selectOption('browser-test-instance')
  await expect(page.locator('#config-page-actions')).toContainText('Qwen3 8B Chat')
  await panel(page).getByRole('button', { name: '退出引导' }).click()
  await page.locator('[data-nav-id="config"]').click()
  await expect(page.locator('#config-page-actions')).toContainText('Qwen3 VL 7B')
})

test('a new instance can be created through its model picker without any tour overlay', async ({ page }) => {
  await openGuide(page, 'guide-no-instances')
  await reachInstances(page)
  await expect(next(page)).toBeDisabled()
  await page.locator('[data-guide="instance-create"]').click()
  const modal = page.getByRole('dialog', { name: '创建新实例' })
  await modal.getByRole('textbox').first().fill('Guided instance')
  await modal.getByRole('button', { name: '从模型仓库选择' }).click()
  await page.getByRole('button', { name: /Qwen Browser Test Q8_0.gguf/ }).click()
  await modal.getByRole('button', { name: '创建实例', exact: true }).click()
  await expect(modal).toHaveCount(0)
  await expect(panel(page).getByLabel('本次引导的实例').locator('option:checked')).toHaveText('Guided instance')
  await next(page).click()
  await expect(page.locator('#config-page-actions')).toContainText('Guided instance')
  await panel(page).getByRole('button', { name: '已检查配置，继续' }).click()
  await expect(next(page)).toBeDisabled()
  await page.locator('[data-guide="instance-runtime"]').getByRole('button', { name: '启动', exact: true }).click()
  await next(page).click()
  await page.locator('[data-guide="instance-connection"]').click()
  await panel(page).getByRole('button', { name: '完成引导' }).click()
  await expect(panel(page).getByRole('heading', { name: '实例已通过连接测试' })).toBeVisible()
})

test('Escape while filling a creation form exits only the guide and preserves the form', async ({ page }) => {
  await openGuide(page)
  await reachInstances(page)
  await page.locator('[data-guide="instance-create"]').click()
  const modal = page.getByRole('dialog', { name: '创建新实例' })
  await modal.getByRole('textbox').first().fill('Keep my input')
  await page.keyboard.press('Escape')
  await expect(panel(page)).toHaveCount(0)
  await expect(modal.getByRole('textbox').first()).toHaveValue('Keep my input')
})

test('empty inventory stays on the prerequisite and optional downloads return to the same step', async ({ page }) => {
  await openGuide(page, 'guide-empty')
  await startGuide(page)
  await expect(next(page)).toBeDisabled()
  await panel(page).getByRole('button', { name: '还没有模型？前往下载' }).click()
  await expect(page.locator('[data-nav-id="downloads"]')).toHaveAttribute('aria-current', 'page')
  await expect(page.locator('[data-guide-progress]')).toHaveText('第 1 / 6 步')
  await panel(page).getByRole('button', { name: '返回当前步骤' }).click()
  await expect(page.locator('[data-nav-id="model-repo"]')).toHaveAttribute('aria-current', 'page')
  await expect(next(page)).toBeDisabled()
})

test('pause and resume preserve progress and guide reading position', async ({ page }) => {
  await openGuide(page)
  const scroller = page.locator('[data-guide-scroll]')
  await scroller.evaluate(element => { element.scrollTop = 1000 })
  const before = await scroller.evaluate(element => element.scrollTop)
  await startGuide(page)
  await next(page).click()
  await panel(page).getByRole('button', { name: '稍后继续' }).click()
  await expect(panel(page)).toHaveCount(0)
  await expect.poll(() => scroller.evaluate(element => element.scrollTop)).toBeGreaterThan(before - 100)
  await page.getByRole('button', { name: '继续上次引导' }).click()
  await expect(page.locator('[data-guide-progress]')).toHaveText('第 2 / 6 步')
  await expect(page.locator('[data-nav-id="engine"]')).toHaveAttribute('aria-current', 'page')
})

test('missing targets show a retry state instead of silently skipping steps', async ({ page }) => {
  await openGuide(page)
  const style = await page.addStyleTag({ content: '[data-guide="model-directories"] { display: none !important; }' })
  await startGuide(page)
  await expect(panel(page).getByRole('button', { name: '重新定位' })).toBeVisible({ timeout: 8000 })
  await expect(page.locator('[data-guide-progress]')).toHaveText('第 1 / 6 步')
  await expect(next(page)).toBeDisabled()
  await style.evaluate(element => element.parentNode?.removeChild(element))
  await panel(page).getByRole('button', { name: '重新定位' }).click()
  await expect(next(page)).toBeEnabled()
})

test('failed connection does not complete the guide', async ({ page }) => {
  await openGuide(page, 'docs-screenshots&connection=fail')
  await reachVerification(page)
  await page.locator('[data-guide="instance-connection"]').click()
  await expect(page.getByText(/Connection refused/).first()).toBeVisible()
  await expect(panel(page).getByRole('button', { name: '完成引导' })).toBeDisabled()
})

test('unsaved configuration blocks navigation and survives exiting the guide', async ({ page }) => {
  await openGuide(page)
  await reachInstances(page)
  await panel(page).getByLabel('本次引导的实例').selectOption('browser-test-instance')
  await next(page).click()
  const temperature = page.locator('[data-config-field="temp"] input')
  await temperature.fill('0.71')
  await expect(panel(page).getByRole('button', { name: '已检查配置，继续' })).toBeDisabled()
  await expect(panel(page).getByRole('button', { name: '上一步' })).toBeDisabled()
  await expect(panel(page).getByLabel('本次引导的实例')).toBeDisabled()
  await page.keyboard.press('Escape')
  await expect(panel(page)).toHaveCount(0)
  await expect(page.locator('[data-nav-id="config"]')).toHaveAttribute('aria-current', 'page')
  await expect(temperature).toHaveValue('0.71')
})

test('connection success cannot be reused for another instance', async ({ page }) => {
  await openGuide(page)
  await reachVerification(page)
  await page.locator('[data-guide="instance-connection"]').click()
  await expect(panel(page).getByRole('button', { name: '完成引导' })).toBeEnabled()
  await panel(page).getByLabel('本次引导的实例').selectOption('browser-stopped-instance')
  await expect(panel(page).getByRole('button', { name: '完成引导' })).toBeDisabled()
  await panel(page).getByLabel('本次引导的实例').selectOption('browser-test-instance')
  await expect(panel(page).getByRole('button', { name: '完成引导' })).toBeDisabled()
})

test('returning from a side trip cannot discard configuration edits', async ({ page }) => {
  await openGuide(page)
  await page.locator('[data-nav-id="instances"]').click()
  await page.getByRole('button', { name: '配置参数', exact: true }).click()
  await page.locator('[data-nav-id="guide"]').click()
  await startGuide(page)
  await page.locator('[data-nav-id="config"]').click()
  await page.locator('[data-config-field="temp"] input').fill('0.72')
  await expect(panel(page).getByRole('button', { name: '返回当前步骤' })).toBeDisabled()
  await expect(panel(page).getByRole('button', { name: '还没有模型？前往下载' })).toBeDisabled()
  await expect(panel(page)).toContainText('请先保存或撤销本次修改')
})

test('a failed retest invalidates an earlier successful connection', async ({ page }) => {
  await openGuide(page)
  await reachVerification(page)
  await page.locator('[data-guide="instance-connection"]').click()
  await expect(panel(page).getByRole('button', { name: '完成引导' })).toBeEnabled()
  await page.evaluate(() => history.replaceState(null, '', '?scenario=docs-screenshots&connection=fail'))
  await page.locator('[data-guide="instance-connection"]').click()
  await expect(page.getByText(/Connection refused/).first()).toBeVisible()
  await expect(panel(page).getByRole('button', { name: '完成引导' })).toBeDisabled()
})

test('advanced tour is separate, localized and stays inside a compact viewport', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 720 })
  await openGuide(page, 'docs-screenshots', 'en-US')
  await page.getByRole('button', { name: 'Explore advanced features' }).click()
  for (let index = 1; index <= 7; index++) {
    await expect(page.locator('[data-guide-progress]')).toHaveText(`Step ${index} of 7`)
    await expect(panel(page)).toBeInViewport()
    await panel(page).getByRole('button', { name: index === 7 ? 'Complete guide' : 'Next', exact: true }).click()
  }
  await expect(panel(page).getByRole('heading', { name: 'Advanced tour completed' })).toBeVisible()
})


test('projectors and importance matrices do not satisfy the model prerequisite', async ({ page }) => {
  await openGuide(page, 'guide-assets-only')
  await startGuide(page)
  await expect(next(page)).toBeDisabled()
  await expect(panel(page)).toContainText('至少一个 GGUF 模型')
})
