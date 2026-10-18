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

  await expect(page.getByRole('heading', { name: '性能监控', level: 1 })).toBeVisible()
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

for (const tab of ['perf', 'bigscreen']) {
  test(`${tab} separates prefill, decode, phase averages, idle and unavailable after reconnect`, async ({ page }) => {
    await page.addInitScript(tab => {
      localStorage.setItem('lang', 'zh-CN')
      localStorage.setItem('lastTab', tab)
    }, tab)
    await page.goto('/?scenario=monitoring')
    const panel = page.locator('[data-inference-throughput]').first()
    await expect(panel).toContainText('Prefill 输入速度')
    await expect(panel).toContainText('Decode 生成速度')
    await expect(panel).toContainText('25.8 tok/s')
    const update = async (mode: string) => page.evaluate(async mode => {
      const now = Date.now()
      const idle = { value: 0, state: 'idle', source: 'none', sampledAt: null, windowMs: null, covered: 0, expected: 0 }
      const live = { value: 1200, state: 'live', source: 'slots', sampledAt: now, windowMs: 3000, covered: 1, expected: 1 }
      const sampling = { ...live, value: null, state: 'sampling', covered: 0 }
      const inference = mode === 'legacy' ? undefined : {
        prefill: mode === 'idle' ? idle : mode === 'restored' ? sampling : live,
        decode: idle,
        tasks: mode === 'idle' ? [] : [{ slotId: 0, taskId: 42, phase: 'prefill', startedAt: now - 5000,
          processedTokens: 6000, generatedTokens: 0, cachedTokens: 10000, progress: 0.6,
          prefillAverage: 900, decodeAverage: null, prefill: mode === 'restored' ? sampling : live, decode: idle }],
      }
      await window.__TAURI_BROWSER_TEST__.emitEvent('monitoring-frame', {
        instanceId: 'browser-test-instance', sessionId: 'browser-monitoring-session', sessionStartedAt: now - 120000,
        ts: now, workload: 'inference', state: mode === 'idle' ? 'idle' : 'active', throughput: 999,
        throughputUnit: 'tok/s', outputTokensPerSecond: 999, inputTokensPerSecond: 8888,
        activeRequests: mode === 'idle' ? 0 : 1, queuedRequests: 0, slotCapacity: 1, busySlots: mode === 'idle' ? 0 : 1,
        averageLatencyMs: null, successRate: null, source: 'llama', dataAgeMs: 0, system: null, inference,
      })
    }, mode)
    await update('prefill')
    await expect(panel.locator('[data-phase="prefill"]')).toContainText('1,200 tok/s')
    await expect(panel.locator('[data-phase="decode"]')).toContainText('0 tok/s')
    await expect(page.locator('[data-inference-tasks]')).toContainText('阶段平均 · Prefill 900 tok/s')
    await expect(panel).not.toContainText('900 tok/s')
    await update('restored')
    await expect(panel).toContainText('采样中')
    await expect(panel).not.toContainText('1,200 tok/s')
    await update('idle')
    await expect(panel.locator('[data-phase="prefill"]')).toContainText('0 tok/s')
    await update('legacy')
    await expect(panel).toContainText('数据不可用')
    await expect(panel).not.toContainText('999 tok/s')
    await expect(panel).not.toContainText('8,888 tok/s')
  })
}

for (const tab of ['perf', 'bigscreen']) {
  for (const [language, width] of [['zh-CN', 560], ['en-US', 1280]] as const) {
    test(`${tab} dual throughput layout at ${width}px ${language}`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width, height: 960 })
      await page.addInitScript(({ tab, language }) => {
        localStorage.setItem('lang', language)
        localStorage.setItem('lastTab', tab)
      }, { tab, language })
      await page.goto('/?scenario=monitoring')
      await expect(page.locator('[data-inference-throughput]').first()).toBeVisible()
      await expect(page.locator('[data-inference-trends]')).toBeVisible()
      const overflow = await page.locator('[data-inference-throughput], [data-inference-trends]').evaluateAll(elements => elements.filter(element => element.scrollWidth > element.clientWidth + 2).length)
      expect(overflow).toBe(0)
      await page.screenshot({ path: testInfo.outputPath(`${tab}-${width}-dark.png`), fullPage: true })
      await page.getByRole('button', { name: language === 'zh-CN' ? '切换到明亮模式' : 'Switch to light mode', exact: true }).click()
      await page.screenshot({ path: testInfo.outputPath(`${tab}-${width}-light.png`), fullPage: true })
    })
  }
}
