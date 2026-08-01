import { chromium } from '@playwright/test'
import { spawn } from 'node:child_process'
import { readFile, mkdir } from 'node:fs/promises'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const outputDir = path.join(projectRoot, 'public', 'docs', 'guide')
const baseUrl = 'http://127.0.0.1:4173'
const appUrl = `${baseUrl}/?scenario=docs-screenshots`
const packageJson = JSON.parse(await readFile(path.join(projectRoot, 'package.json'), 'utf8'))
const version = packageJson.version

const pageShots = [
  { id: 'dashboard', file: '01-dashboard.png', target: '[data-guide="dashboard"]' },
  { id: 'model-repo', file: '02-model-repository.png', target: '[data-guide="model-search"]' },
  { id: 'downloads', file: '03-download-manager.png', target: '[data-guide="download-source"]' },
  { id: 'engine', file: '04-engine-manager.png', target: '[data-guide="engine-scan"]' },
  { id: 'instances', file: '05-instance-manager.png', target: '[data-guide="instance-create"]' },
  { id: 'config', file: '06-configuration.png', target: '[data-guide="config-save"]' },
  { id: 'cluster', file: '07-cluster-manager.png', target: '[data-guide="cluster-scan"]' },
  { id: 'proxy', file: '08-instance-routing.png', target: '[data-guide="proxy-overview"]' },
  { id: 'perf', file: '09-performance.png', target: '[data-guide="perf-select"]' },
  { id: 'bigscreen', file: '10-monitoring-wall.png', target: '[data-guide="monitoring-wall"]' },
  { id: 'logs', file: '11-server-logs.png', target: '[data-guide="logs-clear"]' },
]

const flows = [
  {
    file: 'flow-01-first-run.png',
    title: '首次运行',
    englishTitle: 'First Run',
    subtitle: '从本地模型到可启动实例的完整准备流程',
    steps: [
      { image: '02-model-repository.png', title: '扫描模型', description: 'Scan local GGUF models', position: 'left top' },
      { image: '04-engine-manager.png', title: '识别引擎', description: 'Discover llama-server engines', position: 'left top' },
      { image: '05-instance-manager.png', title: '创建实例', description: 'Create a reusable instance', position: 'left top' },
    ],
  },
  {
    file: 'flow-02-start-and-diagnose.png',
    title: '启动与诊断',
    englishTitle: 'Start and Diagnose',
    subtitle: '启动服务后观察资源状态，并用日志定位异常',
    steps: [
      { image: '05-instance-manager.png', title: '启动实例', description: 'Start the selected instance', position: 'left top' },
      { image: '09-performance.png', title: '观察性能', description: 'Review utilization and throughput', position: 'left top' },
      { image: '11-server-logs.png', title: '检查日志', description: 'Inspect server output and errors', position: 'left top' },
    ],
  },
  {
    file: 'flow-03-route-requests.png',
    title: '统一路由',
    englishTitle: 'Route Requests',
    subtitle: '把可用实例纳入规则，再通过 OpenAI / Anthropic 兼容端点访问',
    steps: [
      { image: '05-instance-manager.png', title: '准备实例', description: 'Keep target instances available', position: 'left top' },
      { image: '08-instance-routing.png', title: '配置路由', description: 'Set aliases and routing policy', position: 'left top' },
      { image: '08-instance-routing.png', title: '调用统一端点', description: 'Use one compatible API endpoint', position: '37% top' },
    ],
  },
]

const delay = (ms) => new Promise(resolve => setTimeout(resolve, ms))

async function serverResponds() {
  try {
    const response = await fetch(baseUrl, { signal: AbortSignal.timeout(1_000) })
    return response.ok
  } catch {
    return false
  }
}

async function waitForServer(serverProcess) {
  const deadline = Date.now() + 120_000
  while (Date.now() < deadline) {
    if (await serverResponds()) return
    if (serverProcess?.exitCode != null) {
      throw new Error(`browser-test server exited with code ${serverProcess.exitCode}`)
    }
    await delay(250)
  }
  throw new Error(`browser-test server did not become ready at ${baseUrl}`)
}

function startServer() {
  const command = process.platform === 'win32' ? 'npm.cmd' : 'npm'
  const child = spawn(command, ['run', 'dev:browser-test'], {
    cwd: projectRoot,
    env: process.env,
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })
  child.stdout.on('data', chunk => process.stdout.write(chunk))
  child.stderr.on('data', chunk => process.stderr.write(chunk))
  return child
}

async function waitForStableLayout(page) {
  await page.evaluate(async () => {
    await document.fonts.ready
    document.querySelectorAll('main > div').forEach(element => {
      if (element instanceof HTMLElement && getComputedStyle(element).overflowY === 'auto') {
        element.scrollTop = 0
      }
    })
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))
  })
  await page.waitForTimeout(180)
  await page.mouse.move(6, 6)
  await page.waitForTimeout(120)
}

async function assertCaptureState(page, expectedId) {
  const state = await page.evaluate(() => ({
    marker: document.documentElement.dataset.tauriBrowserTest,
    unhandled: document.documentElement.dataset.tauriMockUnhandled,
    activeIds: [...document.querySelectorAll('[data-nav-id][aria-current="page"]')]
      .map(element => element.getAttribute('data-nav-id')),
    hoveredIds: [...document.querySelectorAll('[data-nav-id]:hover')]
      .map(element => element.getAttribute('data-nav-id')),
  }))
  if (state.marker !== '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__') {
    throw new Error('documentation capture did not load the isolated browser-test backend')
  }
  if (state.unhandled !== '[]') {
    throw new Error(`documentation capture has unhandled Tauri calls: ${state.unhandled}`)
  }
  if (state.activeIds.length !== 1 || state.activeIds[0] !== expectedId) {
    throw new Error(`expected one active navigation item (${expectedId}), received ${state.activeIds.join(', ')}`)
  }
  if (state.hoveredIds.length > 0) {
    throw new Error(`navigation hover residue detected: ${state.hoveredIds.join(', ')}`)
  }
}

async function seedLogs(page) {
  await page.evaluate(async () => {
    const lines = [
      'main: build = b10042 (6d2f8e1) with CUDA 12.4',
      'load_tensors: loading model tensors, this can take a moment...',
      'load_tensors: offloaded 33/33 layers to GPU',
      'llama_context: n_ctx = 32768, n_batch = 2048, flash_attn = enabled',
      'server: model loaded successfully',
      'server: listening on http://127.0.0.1:18081',
      'slot update_slots: id 0 | task 18 | prompt processing progress 100%',
      'slot update_slots: id 0 | task 18 | generated 256 tokens at 47.6 tok/s',
    ]
    await window.__TAURI_BROWSER_TEST__.emitEvent('server-log-batch', {
      instanceId: 'browser-test-instance',
      lines,
    })
    await window.__TAURI_BROWSER_TEST__.emitEvent('health-status', {
      instanceId: 'browser-test-instance',
      status: 'ok',
    })
    await window.__TAURI_BROWSER_TEST__.emitEvent('download-progress', {
      queueId: 'docs-download-active',
      taskId: 'docs-download-task-active',
      runId: 'docs-download-run-active',
      version: 3,
      fileName: 'Qwen3-14B-Q4_K_M.gguf',
      repoId: 'Qwen/Qwen3-14B-GGUF',
      source: 'huggingface',
      remotePath: 'Qwen3-14B-Q4_K_M.gguf',
      downloaded: 5_368_709_120,
      total: 9_124_839_936,
      speed: 18_874_368,
    })
  })
}

function flowMarkup(flow, images) {
  const cards = flow.steps.map((step, index) => `
    <article class="card">
      <img src="${images.get(step.image)}" style="object-position:${step.position}" alt="" />
      <div class="shade"></div>
      <div class="step">${index + 1}</div>
      <div class="caption">
        <div class="stepTitle">${step.title}</div>
        <div class="description">${step.description}</div>
      </div>
    </article>`).join('')

  return `<!doctype html>
  <html lang="zh-CN"><head><meta charset="utf-8"><style>
    *{box-sizing:border-box} html,body{margin:0;width:3840px;height:2400px;overflow:hidden}
    body{font-family:"Microsoft YaHei UI","Noto Sans SC","Segoe UI",sans-serif;color:#f8fafc;background:
      radial-gradient(circle at 84% 8%,rgba(37,99,235,.2),transparent 30%),
      radial-gradient(circle at 12% 90%,rgba(14,165,233,.11),transparent 32%),#07101f}
    .header{height:330px;padding:102px 112px 0;position:relative}
    h1{font-size:84px;line-height:1.05;letter-spacing:-2px;margin:0;font-weight:800}
    h1 span{color:#94a3b8;font-weight:700}.subtitle{font-size:34px;color:#94a3b8;margin-top:28px}
    .version{position:absolute;right:112px;top:154px;font-size:28px;color:#93c5fd}
    .grid{display:grid;grid-template-columns:repeat(3,1fr);gap:40px;height:1950px;padding:0 112px 112px}
    .card{position:relative;overflow:hidden;border-radius:28px;border:2px solid rgba(100,116,139,.72);background:#020617;box-shadow:0 34px 90px rgba(0,0,0,.34)}
    .card img{width:100%;height:100%;object-fit:cover;display:block}
    .shade{position:absolute;inset:0;background:linear-gradient(to bottom,rgba(2,6,23,.04) 36%,rgba(2,6,23,.28) 58%,rgba(2,6,23,.98) 100%)}
    .step{position:absolute;top:44px;left:44px;width:112px;height:112px;border-radius:999px;display:grid;place-items:center;font-size:54px;font-weight:800;background:#2563eb;border:10px solid rgba(147,197,253,.2);box-shadow:0 14px 44px rgba(37,99,235,.5)}
    .caption{position:absolute;left:58px;right:58px;bottom:64px}.stepTitle{font-size:54px;line-height:1.2;font-weight:800;letter-spacing:-1px}.description{font-size:28px;color:#cbd5e1;margin-top:18px}
  </style></head><body>
    <header class="header"><h1>${flow.title} <span>/ ${flow.englishTitle}</span></h1><div class="subtitle">${flow.subtitle}</div><div class="version">Llama Server Manager v${version}</div></header>
    <main class="grid">${cards}</main>
  </body></html>`
}

async function validatePng(filePath, expectedWidth, expectedHeight) {
  const bytes = await readFile(filePath)
  if (bytes.length < 150_000) throw new Error(`${path.basename(filePath)} is unexpectedly small (${bytes.length} bytes)`)
  const width = bytes.readUInt32BE(16)
  const height = bytes.readUInt32BE(20)
  if (width !== expectedWidth || height !== expectedHeight) {
    throw new Error(`${path.basename(filePath)} has ${width}x${height}, expected ${expectedWidth}x${expectedHeight}`)
  }
  return bytes.length
}

await mkdir(outputDir, { recursive: true })
let serverProcess = null
let browser

try {
  if (!(await serverResponds())) serverProcess = startServer()
  await waitForServer(serverProcess)

  browser = await chromium.launch({ headless: true })
  const context = await browser.newContext({
    viewport: { width: 1600, height: 1000 },
    deviceScaleFactor: 2,
    colorScheme: 'dark',
    locale: 'zh-CN',
    timezoneId: 'Asia/Shanghai',
    reducedMotion: 'reduce',
  })
  await context.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'dashboard')
  })
  const page = await context.newPage()
  await page.goto(appUrl, { waitUntil: 'domcontentloaded' })
  await page.waitForFunction(() => document.documentElement.dataset.tauriBrowserTest === '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__')
  await page.locator('[data-guide="dashboard"]').waitFor({ state: 'visible' })
  await seedLogs(page)

  for (const shot of pageShots) {
    if (shot.id === 'config') {
      const configureButton = page.getByRole('button', { name: '配置参数', exact: true })
      if (await configureButton.count() !== 1) {
        throw new Error('expected one configuration shortcut for the selected documentation instance')
      }
      await configureButton.click()
    } else {
      const navigation = page.locator(`[data-nav-id="${shot.id}"]`)
      if (await navigation.getAttribute('aria-current') !== 'page') await navigation.click()
    }
    await page.waitForFunction(id => document.querySelector(`[data-nav-id="${id}"]`)?.getAttribute('aria-current') === 'page', shot.id)
    await page.locator(shot.target).waitFor({ state: 'visible' })
    await waitForStableLayout(page)
    await assertCaptureState(page, shot.id)
    await page.screenshot({ path: path.join(outputDir, shot.file), animations: 'disabled' })
    process.stdout.write(`captured ${shot.file}\n`)
  }
  await context.close()

  const images = new Map()
  for (const shot of pageShots) {
    const bytes = await readFile(path.join(outputDir, shot.file))
    images.set(shot.file, `data:image/png;base64,${bytes.toString('base64')}`)
  }

  const posterContext = await browser.newContext({
    viewport: { width: 3840, height: 2400 },
    deviceScaleFactor: 1,
    colorScheme: 'dark',
    locale: 'zh-CN',
    reducedMotion: 'reduce',
  })
  const posterPage = await posterContext.newPage()
  for (const flow of flows) {
    await posterPage.setContent(flowMarkup(flow, images), { waitUntil: 'load' })
    await posterPage.evaluate(() => document.fonts.ready)
    await posterPage.screenshot({ path: path.join(outputDir, flow.file), animations: 'disabled' })
    process.stdout.write(`composed ${flow.file}\n`)
  }
  await posterContext.close()

  const validation = []
  for (const shot of pageShots) {
    validation.push({ file: shot.file, bytes: await validatePng(path.join(outputDir, shot.file), 3200, 2000) })
  }
  for (const flow of flows) {
    validation.push({ file: flow.file, bytes: await validatePng(path.join(outputDir, flow.file), 3840, 2400) })
  }
  process.stdout.write(`${JSON.stringify(validation, null, 2)}\n`)
} finally {
  await browser?.close()
  if (serverProcess && serverProcess.exitCode == null) serverProcess.kill()
}
