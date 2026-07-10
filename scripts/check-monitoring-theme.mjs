import fs from 'node:fs'
import path from 'node:path'

const repoRoot = process.cwd()

const files = [
  'src/components/PerformancePage/PerformancePage.tsx',
  'src/components/BigScreenPage.tsx',
  'src/components/monitoring/MonitoringPrimitives.tsx',
]

const hardDarkTokenPatterns = [
  /^bg-slate-(?:900|950)(?:\/\d+)?$/,
  /^border-slate-(?:700|800)(?:\/\d+)?$/,
  /^text-slate-(?:100|200)$/,
]

const allowedOpacity = new Set(['0', '5', '10', '20', '25', '30', '40', '50', '60', '70', '75', '80', '90', '95', '100'])
const failures = []

function tokenize(content) {
  return content.match(/[A-Za-z0-9_!:[\]./%-]+/g) ?? []
}

for (const relativePath of files) {
  const absolutePath = path.join(repoRoot, relativePath)
  const content = fs.readFileSync(absolutePath, 'utf8')

  if (/className=["'`{]*dark(?:\s|["'`}]|$)/.test(content)) {
    failures.push(`${relativePath}: page-local .dark wrapper found; monitoring pages must follow the global theme.`)
  }

  for (const token of tokenize(content)) {
    const classToken = token.replace(/^[:]+|[,;)}\]'"`]+$/g, '')
    if (!classToken || classToken.includes(':')) continue

    if (hardDarkTokenPatterns.some(pattern => pattern.test(classToken))) {
      failures.push(`${relativePath}: hard dark class "${classToken}" is not prefixed with dark:.`)
    }
  }

  for (const token of tokenize(content)) {
    const classToken = token.replace(/^[:]+|[,;)}\]'"`]+$/g, '')
    const baseToken = classToken.split(':').pop() || ''
    const opacity = baseToken.match(/\/(\d+)$/)?.[1]

    if (opacity && !allowedOpacity.has(opacity)) {
      failures.push(`${relativePath}: non-standard opacity class "${classToken}" makes theme auditing fragile.`)
    }
  }
}

if (failures.length > 0) {
  console.error('Monitoring theme check failed:')
  for (const failure of failures) {
    console.error(`- ${failure}`)
  }
  process.exit(1)
}

console.log('Monitoring theme check passed.')
