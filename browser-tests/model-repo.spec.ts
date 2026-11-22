import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('same-level folders, models and projectors align and MTP children have one additional branch', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('lang', 'zh-CN'); localStorage.setItem('lastTab', 'model-repo') })
  await page.goto('/?scenario=model-tree-layout')
  const tree = page.locator('.model-tree-viewport')
  const family = 'C:\\browser-test\\models\\gemma-4-12b-it-GGUF\\'
  const label = (relative: string) => tree.getByTitle(family + relative, { exact: true })
  const siblings = ['MTP', 'gemma-4-12b-it-UD-Q8_K_XL.gguf', 'mmproj-F16.gguf', 'mtp-gemma-4-12b-it.gguf']
  const child = label('MTP\\mtp-gemma-4-12b-it-Q8_0.gguf')
  for (const width of [1640, 560]) {
    await page.setViewportSize({ width, height: 900 })
    await expect(child).toBeVisible()
    const positions = await Promise.all(siblings.map(name => label(name).evaluate(el => ({
      text: el.getBoundingClientRect().left,
      icon: el.previousElementSibling!.getBoundingClientRect().left,
    }))))
    expect(positions.every(pos => pos.text === positions[0].text && pos.icon === positions[0].icon)).toBe(true)
    expect((await child.boundingBox())!.x - positions[0].text).toBe(16)
  }
  const folder = label('MTP').locator('..')
  const branch = child.locator('..').locator('.model-tree-branches > :last-child')
  await expect(branch).toHaveAttribute('data-continues', 'false')
  await folder.click()
  await expect(child).toHaveCount(0)
  await expect(label('mmproj-F16.gguf')).toBeVisible()
  await folder.click()
  await child.locator('..').click()
  await expect(child.locator('..')).toHaveAttribute('aria-pressed', 'true')
  await page.locator('[data-guide=model-search] input').fill('mtp-gemma-4-12b-it-Q8_0')
  await expect(tree.locator('button[data-selected]')).toHaveCount(1)
  await expect(tree.locator('.model-tree-branches > [data-continues=true]')).toHaveCount(0)
})

test('a fresh configuration exposes the default model root and its scanned tree', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'model-repo')
  })
  await page.goto('/')

  await expect(page.locator('html')).toHaveAttribute(
    'data-tauri-browser-test',
    '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__',
  )
  await expect(page.getByText('1 个来源', { exact: true })).toBeVisible()
  const explorer = page.locator('[data-guide="model-search"]')
  await expect(explorer.getByText('C:\\browser-test\\models', { exact: true })).toBeVisible()
  await expect(
    explorer.getByText('Qwen Browser Test Q8_0.gguf', { exact: true }),
  ).toBeVisible()
})

test('a delayed compatible cache hydrates before the background inventory refresh', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'model-repo')
  })
  await page.goto('/?scenario=delayed-inventory-cache')

  const explorer = page.locator('[data-guide="model-search"]')
  await expect(
    explorer.getByText('Qwen Browser Test Q8_0.gguf', { exact: true }),
  ).toBeVisible({ timeout: 1_500 })

  await page.getByRole('button', { name: '引擎管理', exact: true }).click()
  await expect(page.getByText('Browser Test Engine', { exact: true }).first()).toBeVisible({ timeout: 1_000 })
})

test('an empty legacy root list is repaired before a manual scan', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'model-repo')
  })
  await page.goto('/?scenario=empty-model-roots')

  await expect(page.getByText('0 个来源', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '扫描模型', exact: true }).click()

  await expect(page.getByText('1 个来源', { exact: true })).toBeVisible()
  const explorer = page.locator('[data-guide="model-search"]')
  await expect(explorer.getByText('C:\\browser-test\\models', { exact: true })).toBeVisible()
  await expect(explorer.getByText('Qwen Browser Test Q8_0.gguf', { exact: true })).toBeVisible()
  await expect.poll(() => page.evaluate(() => {
    const calls = window.__TAURI_BROWSER_TEST__.calls.filter(call => call.command === 'scan_models')
    return calls[calls.length - 1]?.payload
  })).toEqual({ paths: ['C:\\browser-test\\models'] })
})

test('Windows namespace aliases remain inside ordinary model and engine roots', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'engine')
  })
  await page.goto('/?scenario=windows-path-aliases')

  await expect(page.getByText('1 个已发现', { exact: true })).toBeVisible()
  await expect(page.getByText('Browser Test Engine', { exact: true }).first()).toBeVisible()
  await expect(page.getByTitle('C:\\browser-test\\engine\\build').first()).toBeVisible()

  await page.getByRole('button', { name: '模型仓库', exact: true }).click()
  const explorer = page.locator('[data-guide="model-search"]')
  await expect(explorer.getByText('C:\\browser-test\\models', { exact: true })).toBeVisible()
  await expect(explorer.getByText('Qwen Browser Test Q8_0.gguf', { exact: true })).toBeVisible()
  await expect(explorer.getByTitle('C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0.gguf')).toBeVisible()
  await expect(page.locator('aside').getByTitle('C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0.gguf')).toBeVisible()

  const leakedNamespacePaths = await page.evaluate(() => {
    const namespacePrefix = ['\\', '\\', '?', '\\'].join('')
    const visibleText = document.body.innerText.includes(namespacePrefix) ? ['body text'] : []
    const titles = [...document.querySelectorAll<HTMLElement>('[title]')]
      .map(element => element.title)
      .filter(title => title.includes(namespacePrefix))
    return [...visibleText, ...titles]
  })
  expect(leakedNamespacePaths).toEqual([])
})

test('download storage paths use the readable Windows form', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'downloads')
    localStorage.setItem('downloadSaveDir', '\\\\?\\c:\\browser-test\\downloads\\')
  })
  await page.goto('/')

  await expect(page.locator('[data-guide="download-save-dir"] input')).toHaveValue('C:\\browser-test\\downloads')
  await expect(page.getByTitle('C:\\browser-test\\downloads\\<repo>\\<file>')).toBeVisible()
})

test('sharded model deletion previews and removes the complete physical artifact set', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'model-repo')
  })
  await page.goto('/')

  const explorer = page.locator('[data-guide="model-search"]')
  await expect(explorer.getByText('Qwen Browser Test Q8_0.gguf', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '删除', exact: true }).click()
  await expect(page.getByText('Qwen Browser Test Q8_0.gguf', { exact: true })).toHaveCount(0)

  const deletionFlow = await page.evaluate(() => window.__TAURI_BROWSER_TEST__.calls
    .filter(call => [
      'preview_model_deletion',
      'plugin:dialog|message',
      'delete_model_file',
    ].includes(call.command)))
  expect(deletionFlow.map(call => call.command)).toEqual([
    'preview_model_deletion',
    'plugin:dialog|message',
    'delete_model_file',
  ])
  expect(JSON.stringify(deletionFlow[1].payload)).toContain('3')
  expect(JSON.stringify(deletionFlow[1].payload)).toContain('12.00 GB')
})
