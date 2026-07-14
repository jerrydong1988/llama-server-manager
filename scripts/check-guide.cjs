const fs = require('fs')
const path = require('path')

const ROOT = path.resolve(__dirname, '..')
const GUIDE_PATH = path.join(ROOT, 'GUIDE.md')
const README_PATH = path.join(ROOT, 'README.md')
const APP_PATH = path.join(ROOT, 'src', 'App.tsx')
const APP_SHELL_PATH = path.join(ROOT, 'src', 'components', 'shell', 'AppShell.tsx')
const GUIDE_PAGE_PATH = path.join(ROOT, 'src', 'components', 'GuidePage.tsx')
const TOUR_PATH = path.join(ROOT, 'src', 'components', 'guide', 'guideTour.ts')
const UI_PATH = path.join(ROOT, 'src', 'components', 'ui.tsx')
const ASSET_DIR = path.join(ROOT, 'public', 'docs', 'guide')

const REQUIRED_SECTIONS = [
  '快速开始 / Quick Start',
  '系统总览 / Dashboard',
  '模型仓库 / Model Repository',
  '下载管理 / Download Manager',
  '引擎管理 / Engine Management',
  '实例管理 / Instance Management',
  '参数配置 / Parameter Configuration',
  '集群管理 / Cluster Management',
  '实例路由 / Instance Routing',
  '性能监控 / Performance Monitoring',
  '监控大屏 / Monitoring Wall',
  '服务器日志 / Server Logs',
  '常见问题 / FAQ',
]

const REQUIRED_IMAGES = [
  '01-dashboard.png',
  '02-model-repository.png',
  '03-download-manager.png',
  '04-engine-manager.png',
  '05-instance-manager.png',
  '06-configuration.png',
  '07-cluster-manager.png',
  '08-instance-routing.png',
  '09-performance.png',
  '10-monitoring-wall.png',
  '11-server-logs.png',
  '12-in-app-guide.png',
  'flow-01-first-run.png',
  'flow-02-start-and-diagnose.png',
  'flow-03-route-requests.png',
]

const REQUIRED_TOUR_TABS = [
  'dashboard',
  'model-repo',
  'downloads',
  'engine',
  'instances',
  'config',
  'cluster',
  'proxy',
  'perf',
  'bigscreen',
  'logs',
]

const errors = new Set()

function readText(filePath, label) {
  if (!fs.existsSync(filePath)) {
    errors.add(`${label} is missing: ${path.relative(ROOT, filePath)}`)
    return ''
  }
  return fs.readFileSync(filePath, 'utf8')
}

function slugifyHeading(title) {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fa5]+/g, '-')
    .replace(/^-|-$/g, '')
}

function collectFiles(directory, extension) {
  if (!fs.existsSync(directory)) return []
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name)
    if (entry.isDirectory()) return collectFiles(entryPath, extension)
    return entry.name.endsWith(extension) ? [entryPath] : []
  })
}

const packageJson = JSON.parse(readText(path.join(ROOT, 'package.json'), 'package.json') || '{}')
const packageLock = JSON.parse(readText(path.join(ROOT, 'package-lock.json'), 'package-lock.json') || '{}')
const cargoToml = readText(path.join(ROOT, 'src-tauri', 'Cargo.toml'), 'Cargo.toml')
const tauriConfig = JSON.parse(
  readText(path.join(ROOT, 'src-tauri', 'tauri.conf.json'), 'tauri.conf.json') || '{}',
)
const guide = readText(GUIDE_PATH, 'GUIDE.md')
const readme = readText(README_PATH, 'README.md')
const app = readText(APP_PATH, 'src/App.tsx')
const appShell = readText(APP_SHELL_PATH, 'src/components/shell/AppShell.tsx')
const guidePage = readText(GUIDE_PAGE_PATH, 'src/components/GuidePage.tsx')
const tour = readText(TOUR_PATH, 'src/components/guide/guideTour.ts')
const ui = readText(UI_PATH, 'src/components/ui.tsx')
const expectedVersion = packageJson.version || 'unknown'
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1]
const lockedVersion = packageLock.packages?.['']?.version || packageLock.version
for (const [label, version] of [
  ['package-lock.json', lockedVersion],
  ['Cargo.toml', cargoVersion],
  ['tauri.conf.json', tauriConfig.version],
]) {
  if (version !== expectedVersion) {
    errors.add(`${label} version ${version || 'missing'} must match package.json ${expectedVersion}`)
  }
}

const releaseTag = process.env.GITHUB_REF_TYPE === 'tag'
  ? process.env.GITHUB_REF_NAME
  : process.env.GITHUB_REF?.match(/^refs\/tags\/(.+)$/)?.[1]
if (releaseTag && releaseTag !== `v${expectedVersion}`) {
  errors.add(`Git tag ${releaseTag} must match release version v${expectedVersion}`)
}

if (!guide.includes(`> v${expectedVersion}`)) {
  errors.add(`GUIDE.md must declare release version v${expectedVersion}`)
}

if (guide.includes('Tagged Windows and macOS releases use code signing and notarization')) {
  errors.add('GUIDE.md must not claim every tagged Windows and macOS build is formally signed')
}

for (const marker of ['-unsigned', '-adhoc']) {
  if (!guide.includes(marker)) errors.add(`GUIDE.md must document the ${marker} release fallback`)
}

if (guide.includes('发版前自检 / Release Validation')) {
  errors.add('GUIDE.md must not expose the maintainer-only release validation section')
}

if (guide.includes('#发版前自检-release-validation')) {
  errors.add('GUIDE.md table of contents must not link to release validation')
}

for (const section of REQUIRED_SECTIONS) {
  if (!guide.includes(`## ${section}`)) {
    errors.add(`GUIDE.md is missing section: ${section}`)
  }
}

const headings = new Map()
for (const match of guide.matchAll(/^## (.+)$/gm)) {
  headings.set(slugifyHeading(match[1].trim()), match[1].trim())
}

for (const match of guide.matchAll(/\[[^\]]+\]\(#([^)]+)\)/g)) {
  if (!headings.has(match[1])) {
    errors.add(`GUIDE.md table-of-contents anchor has no heading: #${match[1]}`)
  }
}

const markdown = `${guide}\n${readme}`
const imageReferences = [...markdown.matchAll(/!\[[^\]]*\]\(([^)]+)\)/g)].map((match) => match[1])
for (const imagePath of imageReferences) {
  if (/^https?:\/\//i.test(imagePath)) continue
  const normalized = imagePath.replace(/\\/g, '/')
  if (!normalized.startsWith('public/docs/guide/') || normalized.includes('..')) {
    errors.add(`Guide image path must stay under public/docs/guide/: ${imagePath}`)
    continue
  }
  if (!fs.existsSync(path.join(ROOT, normalized))) {
    errors.add(`Referenced guide image is missing: ${normalized}`)
  }
}

for (const imageName of REQUIRED_IMAGES) {
  const relativePath = `public/docs/guide/${imageName}`
  const isReferenced = imageReferences.includes(relativePath)
  if (!isReferenced) {
    errors.add(`Required guide image is not referenced: ${relativePath}`)
  }
  if (!isReferenced && !fs.existsSync(path.join(ASSET_DIR, imageName))) {
    errors.add(`Required guide image is missing: ${relativePath}`)
  }
}

const navigationTabs = new Set([...app.matchAll(/\bid:\s*['"]([^'"]+)['"]/g)].map((match) => match[1]))
const tourTabs = new Set([...tour.matchAll(/\btab:\s*['"]([^'"]+)['"]/g)].map((match) => match[1]))
const tourSelectors = [...tour.matchAll(/\bselector:\s*['"]\[data-guide=\\?['"]([^'"]+)\\?['"]\]['"]/g)].map((match) => match[1])
const sourceText = collectFiles(path.join(ROOT, 'src'), '.tsx')
  .map((filePath) => fs.readFileSync(filePath, 'utf8'))
  .join('\n')

for (const tab of REQUIRED_TOUR_TABS) {
  if (!navigationTabs.has(tab)) errors.add(`Application navigation is missing tour tab: ${tab}`)
  if (!tourTabs.has(tab)) errors.add(`Guide tour is missing tab: ${tab}`)
}

for (const selector of tourSelectors) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const count = (sourceText.match(new RegExp(`data-guide\\s*=\\s*["']${escaped}["']`, 'g')) || []).length
  if (count === 0) errors.add(`Guide tour selector has no target: ${selector}`)
  if (count > 1) errors.add(`Guide tour selector must be unique: ${selector} (${count} targets)`)
}

if (!/export function Surface\([\s\S]*?\.\.\.elementProps[\s\S]*?<Component[^>]*\{\.\.\.elementProps\}/.test(ui)) {
  errors.add('Surface must forward DOM attributes so data-guide tour targets are rendered')
}

if (!guidePage.includes('onDoneClick')) {
  errors.add('Interactive guide must explicitly complete each single-step driver popover')
}

for (const [token, message] of [
  ['stripInAppTableOfContents', 'In-app guide must hide the duplicated static Markdown table of contents'],
  ['data-guide-scroll', 'In-app guide must expose a dedicated content scroll container'],
  ['data-guide-toc', 'In-app guide must expose an independently scrollable chapter list'],
  [
    "aria-current={activeSectionId === item.id ? 'location' : undefined}",
    'In-app guide must identify the active chapter',
  ],
  ['aria-expanded={isChecklistOpen}', 'In-app guide setup checklist must be collapsible'],
  ['scrollToTop', 'In-app guide must provide a back-to-top action'],
]) {
  if (!guidePage.includes(token)) errors.add(message)
}

if (guidePage.includes('element.scrollIntoView')) {
  errors.add('Guide chapter jumps must not scroll ancestor containers')
}

if (!guidePage.includes('const targetTop = container.scrollTop')) {
  errors.add('Guide chapter jumps must target the dedicated content scroller')
}

if (guidePage.includes('id="guide-setup-checklist" className="mb-4 max-h-')) {
  errors.add('Expanded setup checklist must not clip items inside a fixed-height scroller')
}

if (!guidePage.includes("isChecklistOpen ? 'overflow-y-auto' : 'overflow-hidden'")) {
  errors.add('Guide sidebar must scroll as a whole only when the expanded checklist needs more room')
}

if (!guidePage.includes('id="guide-setup-checklist" className="mb-3 shrink-0 pr-1"')) {
  errors.add('Expanded setup checklist spacing must avoid a redundant sidebar scrollbar at common heights')
}

if (!app.includes("constrainContent={activeTab === 'guide'}")) {
  errors.add('App must opt the guide into a viewport-constrained content layout')
}

if (!appShell.includes("constrainContent ? 'flex h-full min-h-0 flex-col'")) {
  errors.add('App shell must constrain guide content so its internal scrollers can work')
}

if (errors.size > 0) {
  console.error(`Guide check failed with ${errors.size} issue(s):`)
  for (const error of errors) console.error(`- ${error}`)
  process.exit(1)
}

console.log('Guide check passed')
