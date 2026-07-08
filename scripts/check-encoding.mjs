import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { TextDecoder } from 'node:util'

const root = process.cwd()
const includeExt = new Set([
  '.css',
  '.json',
  '.md',
  '.rs',
  '.toml',
  '.ts',
  '.tsx',
  '.yaml',
  '.yml',
])
const skipDirs = new Set([
  '.git',
  'dist',
  'node_modules',
  'src-tauri/target',
])

const decoder = new TextDecoder('utf-8', { fatal: true })
const failures = []
const nonAsciiComments = []

function extOf(path) {
  const match = path.match(/\.[^.\\/]+$/)
  return match ? match[0].toLowerCase() : ''
}

function shouldSkipDir(path) {
  const normalized = relative(root, path).replace(/\\/g, '/')
  return skipDirs.has(normalized)
}

function walk(dir) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    const stats = statSync(path)
    if (stats.isDirectory()) {
      if (!shouldSkipDir(path)) walk(path)
      continue
    }
    if (!includeExt.has(extOf(path))) continue

    try {
      const content = decoder.decode(readFileSync(path))
      checkComments(path, content)
    } catch {
      failures.push(relative(root, path))
    }
  }
}

function shouldCheckComments(path) {
  const normalized = relative(root, path).replace(/\\/g, '/')
  return normalized.startsWith('src-tauri/src/')
    || normalized === 'src/validators.ts'
}

function checkComments(path, content) {
  if (!shouldCheckComments(path)) return
  const lines = content.split(/\r?\n/)
  for (let index = 0; index < lines.length; index += 1) {
    const pos = lines[index].indexOf('//')
    if (pos < 0) continue
    const comment = lines[index].slice(pos)
    if (/[^\x00-\x7F]/.test(comment)) {
      nonAsciiComments.push(`${relative(root, path)}:${index + 1}`)
    }
  }
}

walk(root)

if (failures.length > 0) {
  console.error('Invalid UTF-8 files:')
  for (const file of failures) console.error(`- ${file}`)
  process.exit(1)
}

if (nonAsciiComments.length > 0) {
  console.error('Non-ASCII comments found:')
  for (const item of nonAsciiComments) console.error(`- ${item}`)
  process.exit(1)
}

console.log('UTF-8 check passed')
