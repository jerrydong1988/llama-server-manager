import { expect, test } from '@playwright/test'
import { buildUsageTimeline, DAY, rankUsage, utcDate, type UsageGroup } from '../src/components/routerUsage/chartData'
import { emptyChartSummary } from './routerUsageChartsMock'

test('UTC chart buckets preserve totals, gaps, known zero and the exclusive end', () => {
  const from = Date.parse('2026-01-01T00:00:00Z')
  const days: UsageGroup[] = Array.from({ length: 366 }, (_, i) => ({ id: String(from + i * DAY), name: '',
    summary: { ...emptyChartSummary(), requests: i + 1, input: i * 100, output: i, inputKnown: 1, outputKnown: 1 } }))
  const long = buildUsageTimeline(days, from, from + 365 * DAY)
  expect(long.bucketDays).toBe(30)
  expect(long.buckets).toHaveLength(13)
  expect(long.buckets[12].to - long.buckets[12].from).toBe(5 * DAY)
  expect(long.buckets.reduce((sum, b) => sum + b.requests, 0)).toBe(365 * 366 / 2)
  expect(long.buckets.reduce((sum, b) => sum + b.input + b.output, 0)).toBe(364 * 365 / 2 * 101)
  expect(buildUsageTimeline(days, from, from + 32 * DAY).bucketDays).toBe(7)
  const sparse = buildUsageTimeline([days[0], { ...days[2], summary: { ...emptyChartSummary(), requests: 2, unknown: 2 } }], from, from + 3 * DAY)
  expect(sparse.buckets.map(b => [b.requests, b.input, b.inputKnown, b.unknown])).toEqual([[1, 0, 1, 0], [0, 0, 0, 0], [2, 0, 0, 2]])
})

test('ranking separates missing tokens from known zero and sorts ties by stable identity', () => {
  const groups: UsageGroup[] = [
    { id: 'unknown', name: '', summary: { ...emptyChartSummary(), requests: 99, unknown: 99 } },
    { id: 'zero', name: '', summary: { ...emptyChartSummary(), requests: 2, inputKnown: 2 } },
    ...['b', 'a'].map(id => ({ id, name: '', summary: { ...emptyChartSummary(), requests: 1, input: 20, inputKnown: 1, partial: 1 } })),
  ]
  expect(rankUsage(groups, 'tokens').map(g => [g.id, g.value])).toEqual([['a', 20], ['b', 20], ['zero', 0], ['unknown', null]])
  expect(rankUsage(groups, 'requests')[0].id).toBe('unknown')
})

test.describe('usage charts', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => { localStorage.setItem('lastTab', 'proxy') })
  })
  test.afterEach(async ({ page }) => {
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
  })

  test('timeline exposes exact values, keyboard navigation, missing data and long ranges', async ({ page }) => {
    await page.goto('/?scenario=proxy-routing&usageCharts=1')
    await page.getByRole('tab', { name: '使用统计' }).click()
    const trend = page.getByTestId('usage-trend-chart')
    const detail = page.getByTestId('usage-trend-detail')
    const today = Math.floor(Date.now() / DAY) * DAY
    const latest = trend.getByRole('button', { name: new RegExp(`^${utcDate(today)} ·`) })
    await latest.focus()
    await page.keyboard.press('ArrowLeft')
    await expect(detail).toContainText(utcDate(today - DAY))
    await expect(detail).toContainText('未报告')
    await page.keyboard.press('ArrowLeft')
    await expect(detail).toContainText('已报告输入 Token: 0')
    await expect(detail).not.toContainText('未报告')
    await page.keyboard.press('ArrowLeft')
    await expect(detail).toContainText('此时段无已记录请求')
    await trend.getByRole('button', { name: '请求数', exact: true }).click()
    await expect(trend.getByRole('button', { name: '请求数', exact: true })).toHaveAttribute('aria-pressed', 'true')
    await page.getByRole('button', { name: '今天', exact: true }).click()
    await expect(trend.getByRole('button', { name: new RegExp(`^${utcDate(today)} ·`) })).toHaveCount(1)
    await page.getByLabel('开始日期（UTC）', { exact: true }).fill(utcDate(today - 364 * DAY))
    await expect(trend).toContainText('每 30 天汇总')
    await expect(trend.getByRole('group', { name: /悬停或点击/ }).getByRole('button')).toHaveCount(13)
  })

  test('outcomes include rejected calls and rankings drill down using the same filters', async ({ page }) => {
    await page.goto('/?scenario=proxy-routing')
    await page.getByRole('tab', { name: '使用统计' }).click()
    const outcomes = page.getByTestId('usage-outcome-chart')
    await expect(outcomes).toContainText('25.0%')
    await expect(outcomes).toContainText('33.3%')
    const rank = page.getByTestId('usage-ranking-chart')
    await expect(rank.getByRole('button', { name: '筛选 WorkBuddy · key-a', exact: true })).toContainText('11,000')
    await expect(rank.getByRole('button', { name: '筛选 =Imported Client · key-b', exact: true })).toContainText('23')
    await rank.getByRole('button', { name: '请求数', exact: true }).click()
    await expect(rank.getByRole('button', { name: '筛选 WorkBuddy · key-a', exact: true })).toContainText('50.0%')
    await rank.getByRole('button', { name: '筛选 WorkBuddy · key-a', exact: true }).click()
    await expect(page.getByRole('combobox', { name: '使用统计 API Key' })).toHaveValue('key-a')
    await expect(rank.getByRole('button', { name: '请求数', exact: true })).toHaveAttribute('aria-pressed', 'true')
    await expect(rank.getByRole('button', { name: '筛选 WorkBuddy · key-a', exact: true })).toContainText('100.0%')
    await page.getByRole('combobox', { name: '使用统计 API Key' }).selectOption('')
    await rank.getByRole('combobox', { name: '排行维度' }).selectOption('models')
    await rank.getByRole('button', { name: '筛选 public-model · public-model', exact: true }).click()
    await expect(page.getByRole('combobox', { name: '使用统计 模型' })).toHaveValue('public-model')
    await rank.getByRole('combobox', { name: '排行维度' }).selectOption('instances')
    await rank.getByRole('button', { name: '筛选 instance-one · instance-one', exact: true }).click()
    await expect(page.getByRole('combobox', { name: '使用统计 实例' })).toHaveValue('instance-one')
  })

  test('count-only and empty ranges never show invented token consumption', async ({ page }) => {
    await page.goto('/?scenario=proxy-routing')
    await page.getByRole('tab', { name: '使用统计' }).click()
    await page.getByRole('combobox', { name: '调用类型' }).selectOption('count')
    const trend = page.getByTestId('usage-trend-chart')
    await expect(trend).toContainText('暂无已报告 Token 用量')
    await expect(page.getByTestId('usage-ranking-chart').getByRole('button', { name: /筛选 Preflight/ })).toContainText('未报告')
    await trend.getByRole('button', { name: '请求数', exact: true }).click()
    await expect(page.getByTestId('usage-trend-detail')).toContainText('请求数: 3')
    const tomorrow = utcDate(Date.now() + DAY)
    await page.getByLabel('结束日期（UTC，含当天）', { exact: true }).fill(tomorrow)
    await page.getByLabel('开始日期（UTC）', { exact: true }).fill(tomorrow)
    for (const id of ['usage-trend-chart', 'usage-outcome-chart', 'usage-ranking-chart']) {
      await expect(page.getByTestId(id)).toContainText('所选范围暂无调用记录')
    }
  })

  for (const mode of ['light', 'dark', 'english'] as const) test(`${mode} charts fit and remain readable with multiple callers`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: mode === 'dark' ? 1040 : 1560, height: 1100 })
    await page.addInitScript(mode => { localStorage.setItem('lang', mode === 'english' ? 'en-US' : 'zh-CN') }, mode)
    await page.goto('/?scenario=proxy-routing&usageCharts=1')
    await page.getByRole('tab', { name: mode === 'english' ? 'Usage statistics' : '使用统计' }).click()
    if (mode !== 'dark') await page.getByRole('button', { name: mode === 'english' ? 'Switch to light mode' : '切换到明亮模式' }).click()
    await expect(page.locator('html')).toHaveClass(mode === 'dark' ? /dark/ : /^(?!.*dark)/)
    const rank = page.getByTestId('usage-ranking-chart')
    await expect(rank).toContainText(mode === 'english' ? 'Showing the top 6' : '展示前 6 项')
    await page.getByTestId('usage-trend-chart').evaluate(el => el.scrollIntoView({ block: 'start' }))
    await page.screenshot({ path: testInfo.outputPath(`usage-charts-${mode}.png`) })
    await rank.scrollIntoViewIfNeeded()
    await page.screenshot({ path: testInfo.outputPath(`usage-ranking-${mode}.png`) })
    for (const id of ['usage-trend-chart', 'usage-outcome-chart', 'usage-ranking-chart']) {
      expect(await page.getByTestId(id).evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  })
})
