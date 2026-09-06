import { expect, test } from '@playwright/test'
import type { Page } from '@playwright/test'

async function openConsole(page: Page, multiline = false, language = 'zh-CN') {
  await page.addInitScript(language => {
    localStorage.setItem('lang', language)
    localStorage.setItem('lastTab', 'logs')
  }, language)
  await page.goto('/')
  const console = page.locator('[data-log-console]')
  await expect(console).toBeVisible()
  await page.evaluate(multiline => window.__TAURI_BROWSER_TEST__.emitEvent('server-log-batch', {
    instanceId: 'browser-test-instance',
    lines: Array.from({ length: 600 }, (_, index) => `LOG-${String(index).padStart(4, '0')} server output${multiline && index % 7 === 0 ? '\n  wrapped stack trace: '.repeat(3) : ''}`),
  }), multiline)
  await expect(page.locator('[data-log-row="599"]')).toBeVisible()
  await console.evaluate(element => element.scrollIntoView({ block: 'center' }))
  return console
}

async function dragAcrossPages(page: Page, direction: 'down' | 'up') {
  const console = page.locator('[data-log-console]')
  const startIndex = direction === 'down' ? 0 : 599
  if (direction === 'down') {
    await console.evaluate(element => { element.scrollTop = 0 })
    await expect(page.locator('[data-log-row="0"]')).toBeVisible()
  }
  const start = page.locator(`[data-log-row="${startIndex}"] span`).last()
  const startBox = await start.boundingBox()
  const consoleBox = await console.boundingBox()
  if (!startBox || !consoleBox) throw new Error('console row is unavailable')
  await page.mouse.move(startBox.x + (direction === 'down' ? 1 : 220), startBox.y + 10)
  await page.mouse.down()
  const endY = direction === 'down' ? consoleBox.y + consoleBox.height - 24 : consoleBox.y + 24
  await page.mouse.move(startBox.x + 220, endY, { steps: 12 })
  for (let step = 0; step < 8; step += 1) {
    const before = await console.evaluate(element => element.scrollTop)
    await page.mouse.wheel(0, direction === 'down' ? 460 : -460)
    await expect.poll(() => console.evaluate(element => element.scrollTop)).not.toBe(before)
    // A fresh pointer event extends the native selection into newly rendered rows.
    await page.mouse.move(startBox.x + 219 + step % 2, endY)
  }
  await page.mouse.up()
  return startIndex
}

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('dragging past the console edge selects wrapped logs with native autoscroll', async ({ page }) => {
  const console = await openConsole(page, true)
  await console.evaluate(element => { element.scrollTop = 0 })
  const first = page.locator('[data-log-row="0"] span').last()
  await expect(first).toBeVisible()
  const start = await first.boundingBox()
  const box = await console.boundingBox()
  if (!start || !box) throw new Error('console row is unavailable')
  await page.mouse.move(start.x + 1, start.y + 10)
  await page.mouse.down()
  await page.mouse.move(start.x + 220, box.y + box.height + 25, { steps: 10 })
  await expect.poll(() => page.evaluate(() => (
    window.getSelection()?.toString().match(/LOG-\d{4}/g)?.length ?? 0
  )), { timeout: 15_000 }).toBeGreaterThan(40)
  await page.mouse.up()
  const selected = await page.evaluate(() => window.getSelection()?.toString() ?? '')
  expect(selected).toContain('LOG-0000')
  expect(selected).toContain('wrapped stack trace')
  await expect(page.getByText('选择期间暂停显示更新；取消选择后继续显示，点击“最新”恢复跟随。')).toBeVisible()
  await page.evaluate(() => window.getSelection()?.removeAllRanges())
  await expect.poll(() => page.locator('[data-log-row]').count()).toBeLessThan(100)
})

for (const direction of ['down', 'up'] as const) {
  test(`native log selection survives several pages of ${direction}ward dragging and live log eviction`, async ({ page }) => {
    await openConsole(page, false, direction === 'up' ? 'en-US' : 'zh-CN')
    expect(await page.locator('[data-log-row]').count()).toBeLessThan(100)
    const anchorIndex = await dragAcrossPages(page, direction)
    const selected = await page.evaluate(() => window.getSelection()?.toString() ?? '')
    const selectedLines = selected.match(/LOG-\d{4}/g) ?? []
    expect(selectedLines.length).toBeGreaterThan(80)
    expect(selected).toContain(`LOG-${String(anchorIndex).padStart(4, '0')}`)
    const indexes = selectedLines.map(line => Number(line.slice(4)))
    expect(indexes).toEqual(Array.from({ length: indexes.length }, (_, index) => indexes[0] + index))

    // Exceed both live ring buffers while the original selection remains frozen.
    await page.evaluate(() => window.__TAURI_BROWSER_TEST__.emitEvent('server-log-batch', {
      instanceId: 'browser-test-instance',
      lines: Array.from({ length: 2_500 }, (_, index) => `NEW-${index} streaming output`),
    }))
    await expect.poll(() => page.evaluate(() => window.getSelection()?.toString())).toBe(selected)
    await expect(page.locator('[data-log-console]')).not.toContainText('NEW-')

    // Observe the native copy event without replacing the user's OS clipboard.
    await page.evaluate(() => {
      document.addEventListener('copy', event => {
        document.documentElement.dataset.copiedLogs = window.getSelection()?.toString() ?? ''
        event.preventDefault()
      }, { once: true })
    })
    await page.keyboard.press('ControlOrMeta+c')
    await expect.poll(() => page.locator('html').getAttribute('data-copied-logs')).toBe(selected)

    await page.getByRole('button', { name: direction === 'up' ? 'Latest' : '最新', exact: true }).click()
    await expect(page.locator('[data-log-console]')).toContainText('NEW-2499')
    await expect.poll(() => page.locator('[data-log-row]').count()).toBeLessThan(100)
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe('')
  })
}
