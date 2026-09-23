import { expect, test, type Locator, type Page } from '@playwright/test'

async function openGuideConfig(page: Page, lang = 'zh-CN', scenario = 'docs-screenshots', dark = true) {
  await page.addInitScript(language => {
    localStorage.setItem('lang', language)
    localStorage.setItem('lastTab', 'guide')
  }, lang)
  await page.goto(`/?scenario=${scenario}`)
  const zh = lang === 'zh-CN'
  if (!dark) await page.getByRole('button', { name: zh ? '切换到明亮模式' : 'Switch to light mode', exact: true }).click()
  await expect(page.locator('html')).toHaveClass(dark ? /dark/ : /^(?!.*dark)/)
  await page.getByRole('button', { name: zh ? '开始交互式引导' : 'Start Interactive Tour' }).click()
  const guide = page.locator('[data-guide-panel]')
  const next = guide.getByRole('button', { name: zh ? '下一步' : 'Next', exact: true })
  await next.click()
  await next.click()
  await guide.getByLabel(zh ? '本次引导的实例' : 'Instance for this guide').selectOption('browser-test-instance')
  await next.click()
  await expect(guide.locator('[data-config-guide-actions]')).toBeVisible()
  return guide
}

async function expectUncovered(control: Locator) {
  await expect(control).toBeInViewport({ ratio: 1 })
  await expect.poll(() => control.evaluate(element => {
    const rect = element.getBoundingClientRect()
    return element.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2))
  })).toBe(true)
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

for (const [width, lang, dark] of [
  [1024, 'zh-CN', true], [1024, 'en-US', false],
  [1280, 'zh-CN', false], [1280, 'en-US', true],
  [1920, 'zh-CN', true], [2560, 'en-US', false],
] as const) {
  test(`configuration actions remain usable while scrolling and collapsing at ${width} ${lang}`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 1024 ? 720 : 1000 })
    const guide = await openGuideConfig(page, lang, 'docs-screenshots', dark)
    const zh = lang === 'zh-CN'
    const save = guide.locator('[data-config-floating-save]')
    const proceed = guide.getByRole('button', { name: zh ? '已检查配置，继续' : 'Configuration reviewed, continue' })
    await expect(proceed).toBeEnabled()
    await expectUncovered(save)
    await expectUncovered(proceed)
    await expect(page.locator('[data-config-floating-actions]')).toHaveCount(0)
    await expect(guide.getByRole('combobox')).toHaveCount(0)
    const savesBefore = Number(await page.locator('html').getAttribute('data-tauri-mock-save-count'))
    await page.locator('[data-config-field="temp"] input').fill('0.73')
    await expect(proceed).toBeDisabled()
    await expect(save).toHaveClass(/bg-blue-600/)
    await page.locator('[data-config-field="custom_args"]').scrollIntoViewIfNeeded()
    await expect(page.locator('#config-page-actions')).not.toBeInViewport()
    await expectUncovered(save)
    await expectUncovered(proceed)
    const backToTop = guide.locator('[data-config-back-to-top]')
    await expectUncovered(backToTop)
    await guide.getByRole('button', { name: zh ? '收起引导' : 'Collapse guide' }).click()
    await expectUncovered(save)
    await expectUncovered(proceed)
    await save.click()
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', String(savesBefore + 1))
    await expect(proceed).toBeEnabled()
    await expect(save).not.toHaveClass(/bg-blue-600/)
    expect(await page.evaluate(() => window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance']?.temp)).toBe(0.73)
    await backToTop.click()
    await expect(page.locator('#config-page-actions')).toBeInViewport()
    await expect(backToTop).toHaveCount(0)
    await guide.getByRole('button', { name: zh ? '展开引导' : 'Expand guide' }).click()
    await expectUncovered(proceed)
    await proceed.click()
    await expect(guide.locator('[data-config-guide-actions]')).toHaveCount(0)
    await expect(page.locator('[data-nav-id="instances"]')).toHaveAttribute('aria-current', 'page')
  })
}

test('a failed docked save retains the draft and blocks continuing until retry succeeds', async ({ page }) => {
  const guide = await openGuideConfig(page, 'zh-CN', 'delayed-config-save')
  const input = page.locator('[data-config-field="temp"] input')
  const save = guide.locator('[data-config-floating-save]')
  const proceed = guide.getByRole('button', { name: '已检查配置，继续' })
  await input.fill('0.74')
  await save.click()
  await expect(save).toBeDisabled()
  await expect(proceed).toBeDisabled()
  await expect(guide.getByRole('button', { name: '切换实例', exact: true })).toBeDisabled()
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releaseSave(true))
  await expect(save).toBeEnabled()
  await expect(proceed).toBeDisabled()
  await expect(input).toHaveValue('0.74')
  await expect(guide.getByRole('alert')).toContainText('保存失败，修改已保留')
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '0')
  await save.click()
  await expect(save).toBeDisabled()
  await page.evaluate(() => window.__TAURI_BROWSER_TEST__.releaseSave(false))
  await expect(proceed).toBeEnabled()
  await expect(guide.getByRole('alert')).toHaveCount(0)
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', '1')
})

test('floating actions stay above the guide on side trips and return after exit', async ({ page }) => {
  const guide = await openGuideConfig(page)
  const savesBefore = Number(await page.locator('html').getAttribute('data-tauri-mock-save-count'))
  await guide.getByRole('button', { name: '已检查配置，继续' }).click()
  await page.locator('[data-nav-id="config"]').click()
  await page.locator('[data-config-field="temp"] input').fill('0.75')
  await page.locator('[data-config-field="custom_args"]').scrollIntoViewIfNeeded()
  const floating = page.locator('[data-config-floating-actions]')
  await expect(floating).toBeVisible()
  const assertWithinContent = async () => {
    const actionsRect = await floating.boundingBox()
    const contentRect = await page.locator('[data-workspace-content]').boundingBox()
    expect(actionsRect && contentRect && actionsRect.y + actionsRect.height <= contentRect.y + contentRect.height).toBe(true)
    await expectUncovered(floating.locator('[data-config-floating-save]'))
  }
  await assertWithinContent()
  await guide.getByRole('button', { name: '收起引导' }).click()
  await assertWithinContent()
  await guide.getByRole('button', { name: '退出引导' }).click()
  await expect(guide).toHaveCount(0)
  await assertWithinContent()
  await floating.locator('[data-config-floating-save]').click()
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-save-count', String(savesBefore + 1))
})

test('routing save actions respect the same content boundary during the advanced tour', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 720 })
  await page.addInitScript(() => { localStorage.setItem('lang', 'zh-CN'); localStorage.setItem('lastTab', 'guide') })
  await page.goto('/?scenario=proxy-routing')
  await page.getByRole('button', { name: '进阶功能导览' }).click()
  const guide = page.locator('[data-guide-panel]')
  for (let index = 0; index < 3; index++) await guide.getByRole('button', { name: '下一步', exact: true }).click()
  await page.getByRole('button', { name: '添加 API Key', exact: true }).click()
  const save = page.getByTestId('proxy-floating-save-button')
  await expectUncovered(save)
  const actionsRect = await page.getByTestId('proxy-floating-save').boundingBox()
  const guideRect = await guide.boundingBox()
  expect(actionsRect && guideRect && actionsRect.y + actionsRect.height < guideRect.y).toBe(true)
  await guide.getByRole('button', { name: '收起引导' }).click()
  await expectUncovered(save)
  await save.click()
  await expect(page.getByTestId('proxy-floating-save')).toHaveCount(0)
  await expect(guide).toContainText('第 4 / 7 步')
})
