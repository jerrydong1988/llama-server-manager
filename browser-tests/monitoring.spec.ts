import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('performance monitoring keeps global resources independent from the selected session', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'perf')
  })
  await page.goto('/?scenario=monitoring')

  await expect(page.getByRole('heading', { name: '性能监控' })).toBeVisible()
  const resources = page.locator('section[aria-label="全局系统资源"]')
  await expect(resources).toBeVisible({ timeout: 15_000 })
  await expect(resources).toContainText('9%')
  await expect(resources).toContainText('12%')
  await expect(resources).toContainText('44%')
  await expect(resources).toContainText('75%')
  await expect(resources).not.toContainText('99%')
  await expect(resources).not.toContainText('98%')
  await expect(page.getByText('25.8 tok/s', { exact: true }).first()).toBeVisible()
  const throughputPanel = page.getByRole('heading', { name: '吞吐趋势' }).locator('xpath=ancestor::section[1]')
  const liveLine = throughputPanel.locator('polyline').first()
  await expect(liveLine).toBeVisible()
  const yCoordinates = (await liveLine.getAttribute('points'))
    ?.trim()
    .split(/\s+/)
    .map(point => point.split(',')[1])
  expect(new Set(yCoordinates).size).toBeGreaterThan(1)

  await page.getByRole('button', { name: /Stopped Monitoring Instance/ }).click()
  await expect(page.locator('[data-guide="perf-select"] select')).toHaveValue('browser-stopped-instance')
  await expect(resources).toContainText('9%')
})

test('dashboard exposes CPU, GPU, memory, and VRAM as four global resources', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'dashboard')
  })
  await page.goto('/?scenario=monitoring')

  const resources = page.getByRole('heading', { name: '资源总览' }).locator('xpath=ancestor::section[1]')
  await expect(resources).toContainText('CPU')
  await expect(resources).toContainText('GPU')
  await expect(resources).toContainText('内存')
  await expect(resources).toContainText('显存')
  await expect(resources).toContainText('9%')
  await expect(resources).toContainText('12%')
  await expect(resources).toContainText('44%')
  await expect(resources).toContainText('75%')
})

test('big screen uses the same global resource snapshot and live task throughput', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'bigscreen')
  })
  await page.goto('/?scenario=monitoring')

  await expect(page.getByRole('heading', { name: '大屏模式' })).toBeVisible()
  const resources = page.getByRole('heading', { name: '资源压力' }).locator('xpath=ancestor::section[1]')
  await expect(resources).toContainText('9%')
  await expect(resources).toContainText('12%')
  await expect(resources).toContainText('44%')
  await expect(resources).toContainText('75%')
  await expect(resources).not.toContainText('99%')
  await expect(page.getByText('25.8 tok/s', { exact: true }).first()).toBeVisible()
})
