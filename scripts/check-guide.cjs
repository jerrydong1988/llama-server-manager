const fs = require('fs')
const path = require('path')

const ROOT = path.resolve(__dirname, '..')
const GUIDE_PATH = path.join(ROOT, 'GUIDE.md')
const README_PATH = path.join(ROOT, 'README.md')
const APP_PATH = path.join(ROOT, 'src', 'App.tsx')
const TOUR_PATH = path.join(ROOT, 'src', 'components', 'guide', 'guideTour.ts')
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
const guide = readText(GUIDE_PATH, 'GUIDE.md')
const readme = readText(README_PATH, 'README.md')
const app = readText(APP_PATH, 'src/App.tsx')
const tour = readText(TOUR_PATH, 'src/components/guide/guideTour.ts')
const expectedVersion = packageJson.version || 'unknown'

if (!guide.includes(`> v${expectedVersion}`)) {
  errors.add(`GUIDE.md must declare release version v${expectedVersion}`)
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

if (errors.size > 0) {
  console.error(`Guide check failed with ${errors.size} issue(s):`)
  for (const error of errors) console.error(`- ${error}`)
  process.exit(1)
}

console.log('Guide check passed')
