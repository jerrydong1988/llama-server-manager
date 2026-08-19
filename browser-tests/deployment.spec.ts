import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

const openInstances = async (page: import('@playwright/test').Page, scenario?: string, lang = 'en-US') => {
  await page.addInitScript(({ language }) => {
    localStorage.setItem('lang', language)
    localStorage.setItem('lastTab', 'instances')
  }, { language: lang })
  await page.goto(scenario ? `/?scenario=${scenario}` : '/')
  await expect(page.locator('html')).toHaveAttribute(
    'data-tauri-browser-test',
    '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__',
  )
}

test('ready deployment exposes current, rollback, runtime policy, and revision history', async ({ page }) => {
  await openInstances(page)
  const panel = page.getByTestId('deployment-panel')
  await expect(panel).toContainText('Deployment Revision')
  await expect(panel).toContainText('Ready')
  await expect(panel).toContainText('Current revision')
  await expect(panel).toContainText('Rollback target')
  await expect(panel).toContainText('Runtime policy')
  await expect(panel.getByText('Revision history (1)')).toBeVisible()
})

test('legacy instances explain first materialization without exposing backend detail', async ({ page }) => {
  await openInstances(page, 'deployment-unmaterialized')
  const panel = page.getByTestId('deployment-panel')
  await expect(panel).toContainText('Not materialized')
  await expect(panel).toContainText('The next qualified start will create the first immutable revision.')
  await expect(panel).not.toContainText('browser mock unmaterialized detail')
})

test('stale deployment guidance is localized in Chinese', async ({ page }) => {
  await openInstances(page, 'deployment-stale', 'zh-CN')
  const panel = page.getByTestId('deployment-panel')
  await expect(panel).toContainText('\u9700\u8981\u65B0\u4FEE\u8BA2')
  await expect(panel).toContainText('\u5236\u54C1\u3001\u914D\u7F6E\u3001\u7B56\u7565\u6216\u8DEF\u7531\u5DF2\u53D8\u66F4')
  await expect(panel).not.toContainText('browser mock state detail')
})

test('invalid deployment states make blocked recovery visible', async ({ page }) => {
  await openInstances(page, 'deployment-invalid')
  const panel = page.getByTestId('deployment-panel')
  await expect(panel).toContainText('Invalid')
  await expect(panel).toContainText('Recovery is blocked.')
})
