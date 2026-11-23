import { expect, test, type Page } from '@playwright/test'

async function openPrefill(page: Page, mode: string, lang = 'zh-CN') {
  await page.clock.setFixedTime(Date.now())
  await page.addInitScript((language) => {
    localStorage.setItem('lang', language)
    localStorage.setItem('lastTab', 'perf')
  }, lang)
  await page.goto('/?scenario=monitoring&prefill=' + mode, { waitUntil: 'domcontentloaded' })
  await page.locator('[data-reading=speed]').click()
  await page
    .getByRole('button', { name: lang === 'zh-CN' ? '预填充速度' : 'Prefill speed', exact: true })
    .click()
  return page.locator('.monitor-detail')
}

test('short prompts have individual completion dots and an exact token/time weighted window without live samples', async ({
  page,
}) => {
  const detail = await openPrefill(page, 'replay')
  await expect(page.locator('[data-reading=speed]')).toContainText('上次已完成请求的 Prefill 111.1 tok/s')
  await expect(detail.locator('[data-prefill-summary]')).toContainText('151.38 tok/s')
  await expect(detail.locator('[data-prefill-summary]')).toContainText('975')
  await expect(detail.locator('[data-chart-kind=points] circle')).toHaveCount(3)
  await expect(detail.locator('polyline')).toHaveCount(0)
  await detail.locator('summary').click()
  await expect(detail.getByRole('list').getByText(/#1835.*159.*1,430.94.*111.12/)).toBeVisible()
  await detail.getByRole('button', { name: '任务实时', exact: true }).click()
  await expect(detail).toContainText('暂无采样')
  await detail.getByRole('button', { name: '按请求', exact: true }).click()
  await expect(detail.locator('[data-chart-kind=points] circle')).toHaveCount(3)
  await page.locator('[data-guide=perf-select] select').selectOption('browser-stopped-instance')
  await page.locator('[data-reading=speed]').click()
  await page.getByRole('button', { name: '预填充速度', exact: true }).click()
  await expect(page.locator('.monitor-detail')).toContainText('近五分钟暂无')
  await expect(page.locator('.monitor-detail [data-chart-kind=points]')).toHaveCount(0)
})

test('Gufo reports live wall-clock throughput separately from rounded completed request timings', async ({
  page,
}) => {
  const detail = await openPrefill(page, 'gufo')
  await expect(detail.locator('[data-prefill-summary]')).toContainText('≈1,069.5 tok/s')
  await detail.locator('summary').click()
  await expect(detail.getByRole('list').getByText(/r1292.*1,320/)).toBeVisible()
  await detail.getByRole('button', { name: '实例实时', exact: true }).click()
  await expect(detail).toContainText('两次读取计数器的实际间隔')
  await expect(detail).toContainText('1,536 tok/s')
  await expect(detail.locator('[data-prefill-summary]')).toHaveCount(0)
})

test('llama.cpp instance window uses its compute-time basis in English', async ({ page }) => {
  const detail = await openPrefill(page, 'llama-window', 'en-US')
  await detail.getByRole('button', { name: 'Live instance', exact: true }).click()
  await expect(detail).toContainText('Per engine prefill compute second')
  await expect(detail).toContainText('111.12 tok/s')
  await page.setViewportSize({ width: 390, height: 844 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})
