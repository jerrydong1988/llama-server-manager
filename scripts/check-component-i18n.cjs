const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const targets = [path.join(root, 'src', 'App.tsx')]

function collect(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name)
    if (entry.isDirectory()) collect(fullPath)
    else if (/\.(?:ts|tsx)$/.test(entry.name)) targets.push(fullPath)
  }
}

collect(path.join(root, 'src', 'components'))

const forbidden = [
  { pattern: /lang\s*===\s*['"]zh-CN['"]/g, label: 'component language branch' },
  { pattern: /(?:lang|locale)\.startsWith\(\s*['"]zh['"]\s*\)/g, label: 'component locale branch' },
  { pattern: /\bconst\s+zh\s*=/g, label: 'component-local zh copy selector' },
]

const findings = []
for (const file of targets) {
  const source = fs.readFileSync(file, 'utf8')
  for (const rule of forbidden) {
    for (const match of source.matchAll(rule.pattern)) {
      const line = source.slice(0, match.index).split('\n').length
      findings.push(`${path.relative(root, file)}:${line}: ${rule.label}`)
    }
  }
}

if (findings.length > 0) {
  console.error('Visible locale selection must live under src/i18n:')
  console.error(findings.join('\n'))
  process.exit(1)
}

console.log(`Component i18n boundary check passed (${targets.length} files).`)
