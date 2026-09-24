import { expect, test, type Page, type Locator } from '@playwright/test'
import { exportShellCommand } from '../src/store/commandFormatting'
import { buildCommandPreview } from '../src/components/InstanceManager/commandPreview'

const engine = `C:\\AI Engines\\${'long-engine-folder-'.repeat(8)}\\llama-server.exe`
const model = `C:\\模型目录 with spaces\\${'LongModelName'.repeat(15)}\\Qwen3.8-Q8_0.gguf`
const secret = 'preview-only-api-secret'
const token = 'preview-only-hf-token'
const command = [engine, '-m', model, '-a', `Qwen's "local" model`, '--ctx-size', '32768',
  '-ngl', '999', '--threads', '8', '--temp', '0.7', '--top-k', '40', '--reasoning', 'auto',
  '--spec-type', 'ngram-simple', '--host', '127.0.0.1', '--port', '8080', '--api-key', secret,
  `--hf-token=${token}`, '--metrics', '--metadata', '{"message":"$env:PATH; $(echo literal)"}',
  '--empty=', '--value', '-1.5', '--custom', 'first', '--custom', 'second',
  ...Array.from({ length: 18 }, (_, index) => [`--custom-${index}`, `value-${index}`]).flat(),
  '--', '--literal-value', 'tail with spaces']

async function openPreview(page: Page, language: string, dark: boolean, platform: string) {
  await page.addInitScript(({ language, platform }) => {
    localStorage.setItem('lang', language)
    localStorage.setItem('lastTab', 'instances')
    Object.defineProperty(navigator, 'platform', { value: platform })
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (text: string) => {
      (window as unknown as { previewCopied: string }).previewCopied = text
    } } })
  }, { language, platform })
  await page.goto('/')
  const zh = language === 'zh-CN'
  if (!dark) await page.getByRole('button', { name: zh ? '切换到明亮模式' : 'Switch to light mode', exact: true }).click()
  await page.evaluate(argv => { window.__TAURI_BROWSER_TEST__.commandOverride = argv }, command)
  const opener = page.getByRole('button', { name: zh ? '生成命令' : 'Cmd', exact: true }).last()
  await opener.click()
  return { dialog: page.getByRole('dialog', { name: zh ? '生成的命令行' : 'Generated Command' }), opener }
}

async function expectContained(dialog: Locator) {
  expect(await dialog.evaluate(element => {
    const viewport = { width: innerWidth, height: innerHeight }
    const bounds = element.getBoundingClientRect()
    return bounds.left >= 0 && bounds.right <= viewport.width && bounds.top >= 0 && bounds.bottom <= viewport.height
      && [element, ...element.querySelectorAll('section, dt, dd, pre, code')]
        .every(item => item.scrollWidth <= item.clientWidth + 1)
  })).toBe(true)
}

test('preview preserves literal argv, repeated options and all secret aliases', () => {
  const args = [engine, 'positional value', '--model', model, '--model-draft=draft path.gguf',
    '--api-key', '-looks-like-a-flag', '--hf-token=token=a=b', '-hft', 'other-secret',
    '--custom', '', 'line\nvalue', 'a"b', '--custom=a=b', '--', '--literal', '-1e3']
  const original = [...args]
  const preview = buildCommandPreview(args)
  expect(args).toEqual(original)
  expect(preview.executable).toBe(engine)
  expect(preview.hasSecrets).toBe(true)
  expect(preview.groups.find(group => group.id === 'model')?.rows[0].values).toEqual([model])
  expect(preview.groups.find(group => group.id === 'speculative')?.rows[0].values).toEqual(['draft path.gguf'])
  expect(preview.groups.find(group => group.id === 'other')?.rows.map(row => [row.flag, row.values])).toEqual([
    [null, ['positional value']], ['--custom', ['', 'line\nvalue', 'a"b']], ['--custom', ['a=b']], ['--', ['--literal', '-1e3']],
  ])
  expect(preview.groups.find(group => group.id === 'network')?.rows.map(row => row.values)).toEqual([['********'], ['********'], ['********']])
  expect(buildCommandPreview([engine])).toMatchObject({ executable: engine, groups: [], hasSecrets: false })
})

test.describe('command preview UI', () => {
  test.afterEach(async ({ page }) => {
    await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
  })

  for (const [width, height, language, dark, platform] of [
    [560, 600, 'zh-CN', true, 'Win32'],
    [1024, 720, 'en-US', false, 'Linux x86_64'],
    [1280, 800, 'zh-CN', false, 'Win32'],
    [1920, 1080, 'en-US', true, 'MacIntel'],
  ] as const) {
    test(`command preview groups and wraps literal parameters at ${width}px ${language}`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width, height })
      const { dialog, opener } = await openPreview(page, language, dark, platform)
      const zh = language === 'zh-CN'
      const copy = dialog.getByRole('button', { name: zh ? '复制完整命令' : 'Copy full command', exact: true })
      const start = dialog.getByRole('button', { name: zh ? '直接启动' : 'Start Directly', exact: true })
      const modelGroup = dialog.getByRole('region', { name: zh ? '模型' : 'Model', exact: true })
      await expect(modelGroup.getByText(model, { exact: true })).toBeVisible()
      await expect(modelGroup.getByText(`Qwen's "local" model`, { exact: true })).toBeVisible()
      await expect(dialog.getByRole('region', { name: zh ? '上下文与缓存' : 'Context and cache' })).toContainText('32768')
      await expect(dialog.locator('pre')).toBeHidden()
      expect(await dialog.textContent()).not.toContain(secret)
      expect(await dialog.textContent()).not.toContain(token)
      await expect(copy).toBeInViewport({ ratio: 1 })
      await expect(start).toBeInViewport({ ratio: 1 })
      await expectContained(dialog)
      await page.screenshot({ path: testInfo.outputPath('grouped-command.png') })

      const row = (flag: string) => dialog.locator('[data-command-parameter]').filter({ has: page.locator('dt', { hasText: new RegExp(`^${flag}$`) }) })
      await expect(row('--custom')).toHaveCount(2)
      await expect(row('--empty').locator('dd')).toHaveText('""')
      await expect(row('--value').locator('dd')).toHaveText('-1.5')
      await expect(row('--').locator('dd')).toContainText('--literal-value')
      await dialog.locator('summary').click()
      await expect(dialog.locator('pre')).toBeVisible()
      await dialog.locator('pre').scrollIntoViewIfNeeded()
      await expect(dialog.locator('pre')).toContainText('********')
      await expect(dialog.locator('pre')).not.toContainText(secret)
      await expectContained(dialog)
      await expect(copy).toBeInViewport({ ratio: 1 })
      await expect(start).toBeInViewport({ ratio: 1 })
      await page.screenshot({ path: testInfo.outputPath('expanded-command.png') })

      await copy.click()
      const shell = platform === 'Win32' ? 'powershell' : 'posix'
      expect(await page.evaluate(() => (window as unknown as { previewCopied: string }).previewCopied)).toBe(exportShellCommand(command, shell))
      await expect(dialog.getByRole('status')).toBeVisible()
      await page.keyboard.press('Escape')
      await expect(dialog).toBeHidden()
      await expect(opener).toBeFocused()
      await opener.click()
      await expect(dialog.locator('pre')).toBeHidden()
      await expect(dialog.getByRole('status')).toHaveCount(0)
      await dialog.getByRole('button', { name: zh ? '关闭' : 'Close', exact: true }).focus()
      await page.keyboard.press('Shift+Tab')
      await expect(start).toBeFocused()
      await page.keyboard.press('Tab')
      await expect(dialog.getByRole('button', { name: zh ? '关闭' : 'Close', exact: true })).toBeFocused()
      await start.click()
      await expect(dialog).toBeHidden()
      await expect.poll(() => page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'start_server').length)).toBe(1)
    })
  }
})
