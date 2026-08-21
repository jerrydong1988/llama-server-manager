import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('single-node residency policy previews and warms an exact managed revision', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'en-US')
    localStorage.setItem('lastTab', 'cluster')
  })
  await page.goto('/?scenario=model-residency')
  await expect(page.locator('html')).toHaveAttribute(
    'data-tauri-browser-test',
    '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__',
  )

  const panel = page.getByTestId('residency-panel')
  await expect(panel).toContainText('Automatic model residency')
  await expect(panel).toContainText('Zero-Worker single-node mode is fully supported.')
  await panel.getByTestId('residency-enabled').check()
  await panel.getByTestId('residency-ram-budget').fill('8')
  await panel.getByTestId('residency-intent-browser-test-instance').check()
  await panel.getByRole('button', { name: 'Save and preview' }).click()

  await expect(panel).toContainText('Selected')
  await expect(panel.getByTestId('residency-operations')).toContainText('Warm')
  await panel.getByRole('button', { name: /Apply plan/ }).click()
  await expect(panel).toContainText('Residency plan reconciled.')
  await expect(panel.getByTestId('residency-operations')).toContainText('Runtime state already matches the plan.')

  const calls = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.map(call => call.command))
  expect(calls).toContain('save_model_residency_policy')
  expect(calls).toContain('start_server')
  expect(calls).toContain('complete_model_residency_operation')
})
