import { expect, test } from '@playwright/test'
import manifest from '../package.json' with { type: 'json' }
import baseline from '../scripts/llama-parameter-baseline.json' with { type: 'json' }

const { version } = manifest

for (const language of ['zh-CN', 'en-US']) {
  test(`bundled guide preserves engine/history versions and exposes current help in ${language}`, async ({ page }) => {
    await page.setViewportSize(language === 'zh-CN' ? { width: 1600, height: 1000 } : { width: 1000, height: 800 })
    await page.addInitScript(lang => {
      localStorage.setItem('lang', lang)
      localStorage.setItem('lastTab', 'guide')
    }, language)
    await page.goto('/?scenario=docs-screenshots')
    const content = page.locator('.guide-content')
    await content.waitFor({ state: 'visible' })
    if (language === 'en-US') await page.getByRole('button', { name: 'Switch to light mode', exact: true }).click()
    await expect(content.locator('blockquote').first()).toContainText(`v${version}`)
    await expect(content).toContainText(`current stable upstream ${baseline.upstreamRelease} baseline tracks ${baseline.releaseSnapshot.parameterCount} parameter entries`)
    await expect(content).toContainText('v2.9.35')
    await expect(content).toContainText('v2.9.36')
    await expect(content).toContainText('v2.9.43')
    await expect(content).toContainText('v0.8.0')
    await expect(content).toContainText('v0.7.0')

    for (const title of ['Gufo 实例 / Gufo Instances', '后台任务 / Background Tasks', '存储维护 / Storage Maintenance', 'CLI 与 MCP / CLI and MCP', '帮助与反馈 / Guide and Support']) {
      const chapter = page.locator('[data-guide-toc]').getByRole('button', { name: title, exact: true })
      await chapter.click()
      await expect(content.getByRole('heading', { name: title, exact: true })).toBeInViewport()
      await expect(chapter).toHaveAttribute('aria-current', 'location')
    }
    await expect(content.getByRole('link', { name: '检查点使用与排障 / Checkpoint guide', exact: true }))
      .toHaveAttribute('href', 'https://github.com/jerrydong1988/llama-server-manager/blob/master/docs/KV_CACHE_CHECKPOINT.md')
    await expect(content.getByRole('link', { name: 'CLI/MCP 管理说明', exact: true }))
      .toHaveAttribute('href', 'https://github.com/jerrydong1988/llama-server-manager/blob/master/docs/agent-management-exploration.md')
    const missingLinks = await content.locator('a').evaluateAll(links => links.filter(link => !link.getAttribute('href')).map(link => link.textContent))
    expect(missingLinks).toEqual([])
    await page.screenshot({ path: `test-results/guide-content-${language}.png` })
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
  })
}
