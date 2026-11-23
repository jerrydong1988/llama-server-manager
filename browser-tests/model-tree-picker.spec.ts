import { expect, test, type Locator } from '@playwright/test'

const family = 'C:\\browser-test\\models\\gemma-4-12b-it-GGUF\\'
const mainFile = 'gemma-4-12b-it-UD-Q8_K_XL.gguf'
const nested = 'MTP\\mtp-gemma-4-12b-it-Q8_0.gguf'
const row = (tree: Locator, relative: string) => tree.getByTitle(family + relative, { exact: true }).locator('..')

async function presentation(tree: Locator, relative: string) {
  return row(tree, relative).evaluate(el => {
    const icon = el.querySelector(':scope > svg')!
    const name = icon.nextElementSibling!
    const box = el.getBoundingClientRect()
    return {
      rowHeight: box.height, nameOffset: name.getBoundingClientRect().left - box.left,
      icon: icon.getAttribute('class'), iconWidth: icon.getBoundingClientRect().width,
      branches: [...el.querySelectorAll('.model-tree-branches > span')].map(branch => branch.getAttribute('data-continues')),
      quant: el.querySelector('.model-tree-quant')?.textContent,
      kind: el.querySelector('.model-tree-kind')?.textContent, size: el.querySelector('.model-tree-size')?.textContent,
    }
  })
}

for (const dark of [false, true]) {
  test('the create picker uses the repository rows, search and current selection in ' + (dark ? 'dark' : 'light') + ' mode', async ({ page }) => {
    await page.setViewportSize({ width: 1640, height: 940 })
    await page.addInitScript(() => { localStorage.setItem('lang', 'zh-CN'); localStorage.setItem('lastTab', 'model-repo') })
    await page.goto('/?scenario=model-tree-layout')
    const repository = page.locator('[data-guide=model-search] .model-tree-viewport')
    await expect(row(repository, mainFile)).toBeVisible()
    if (await page.locator('html').evaluate(el => el.classList.contains('dark')) !== dark) {
      await page.getByRole('button', { name: dark ? '切换到深色模式' : '切换到明亮模式' }).click()
    }
    const reference = await presentation(repository, mainFile)
    const nestedReference = await presentation(repository, nested)
    const projectorReference = await presentation(repository, 'mmproj-F16.gguf')
    expect(reference).toMatchObject({ rowHeight: 39, iconWidth: 16, quant: 'Q8_K_XL', kind: '模型', size: '4.00 GB' })
    expect(nestedReference.nameOffset - reference.nameOffset).toBe(16)
    const folderCounts = await row(repository, 'MTP').getAttribute('aria-description')
    expect(folderCounts).toBe('1 模型')

    await page.locator('[data-nav-id=instances]').click()
    await page.locator('[data-guide=instance-create]').click()
    const form = page.locator('[data-guide-dialog=create-instance]')
    const open = form.getByRole('button', { name: '从模型仓库选择' })
    await open.click()
    const dialog = page.getByRole('dialog', { name: '从模型仓库选择' })
    const search = dialog.getByRole('searchbox')
    await expect(search).toBeFocused()
    expect(await presentation(dialog, mainFile)).toEqual(reference)
    expect(await presentation(dialog, nested)).toEqual(nestedReference)
    expect(await presentation(dialog, 'mmproj-F16.gguf')).toEqual(projectorReference)
    const folder = row(dialog, 'MTP')
    await expect(folder).toHaveAttribute('aria-description', folderCounts!)
    await expect(row(dialog, 'mmproj-F16.gguf')).toHaveAttribute('aria-disabled', 'true')
    await row(dialog, 'mmproj-F16.gguf').click()
    await expect(dialog).toBeVisible()

    await folder.click()
    await expect(row(dialog, nested)).toHaveCount(0)
    const headerY = (await search.boundingBox())!.y
    await search.fill('mtp-gemma-4-12b-it-Q8_0')
    await expect(row(dialog, nested)).toBeVisible()
    await expect(dialog.locator('mark')).not.toHaveCount(0)
    await search.fill('no-matching-asset')
    await expect(dialog).toContainText('没有匹配的文件或目录')
    expect((await search.boundingBox())!.y).toBe(headerY)
    await search.fill('')
    await expect(folder).toHaveAttribute('aria-expanded', 'false')

    await row(dialog, mainFile).click()
    await expect(dialog).toHaveCount(0)
    await expect(form.locator('input[readonly]')).toHaveValue(mainFile)
    await open.click()
    const selected = row(dialog, mainFile)
    await expect(selected).toHaveAttribute('aria-pressed', 'true')
    await expect(selected).toHaveCSS('background-color', dark ? 'rgb(26, 48, 40)' : 'rgb(236, 250, 243)')
    await page.setViewportSize({ width: 390, height: 780 })
    await expect(selected).toBeVisible()
    expect(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
    await expect(open).toBeFocused()
    await expect(form.locator('input[readonly]')).toHaveValue(mainFile)
  })
}

test('the configuration picker searches a virtualized library and writes only the requested file field', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('lang', 'zh-CN'); localStorage.setItem('lastTab', 'instances') })
  await page.goto('/?scenario=model-tree-layout')
  await page.getByRole('button', { name: '配置参数', exact: true }).last().click()
  const main = page.locator('[data-config-field="model_path"]')
  const existing = await main.locator('input').inputValue()
  const last = await page.evaluate(async () => {
    const storeModule = '/src/store.ts'
    const { useAppStore } = await import(/* @vite-ignore */ storeModule)
    const { models, modelDirs } = useAppStore.getState()
    const base = models[0]
    const more = Array.from({ length: 500 }, (_, index) => {
      const name = 'library-' + String(index).padStart(3, '0') + '.gguf'
      const path = modelDirs[0] + '\\inventory\\' + name
      return { ...base, id: path, name, path }
    })
    useAppStore.setState({ models: [...models, ...more] })
    return more[more.length - 1].path
  })
  const dialog = page.getByRole('dialog', { name: '从模型仓库选择' })
  await main.locator('button').last().click()
  const viewport = dialog.locator('.model-tree-viewport')
  await expect(viewport.locator('.model-tree-row').first()).toBeVisible()
  expect(await viewport.locator('.model-tree-row').count()).toBeLessThan(100)
  await viewport.evaluate(el => { el.scrollTop = el.scrollHeight })
  await expect(dialog.getByTitle(last, { exact: true })).toBeInViewport()
  await page.keyboard.press('Escape')
  await expect(main.locator('input')).toHaveValue(existing)

  await main.locator('button').last().click()
  await dialog.getByRole('searchbox').fill('library-499')
  await dialog.getByTitle(last, { exact: true }).click()
  await expect(main.locator('input')).toHaveValue(last)
  await main.locator('button').last().click()
  const current = dialog.getByTitle(last, { exact: true }).locator('..')
  await expect(current).toHaveAttribute('aria-pressed', 'true')
  await expect(current).toBeInViewport()
  await page.keyboard.press('Escape')
  await page.getByRole('textbox', { name: '参数搜索' }).fill('--mmproj')
  const projector = page.locator('[data-config-field="mmproj_path"]')
  await projector.locator('button').last().click()
  await expect(dialog.getByRole('button', { name: /library-499/ })).toHaveCount(0)
  await row(dialog, 'mmproj-F16.gguf').click()
  await expect(projector.locator('input')).toHaveValue(family + 'mmproj-F16.gguf')
  await expect(main.locator('input')).toHaveValue(last)
})
