import fs from 'node:fs'
import path from 'node:path'

const relativePath = 'src/components/ClusterPage/ClusterPage.tsx'
const source = fs.readFileSync(path.join(process.cwd(), relativePath), 'utf8')
const warningPattern = /<div className="([^"]+)">\s*\{t\.clusterPage\.sshWarning\}\s*<\/div>/g
const warningClasses = [...source.matchAll(warningPattern)].map(match => match[1].split(/\s+/))
const failures = []

const requiredClasses = [
  'border-amber-200',
  'bg-amber-50',
  'text-amber-800',
  'dark:border-amber-500/20',
  'dark:bg-amber-500/10',
  'dark:text-amber-200',
]

if (warningClasses.length !== 2) {
  failures.push(`${relativePath}: expected 2 SSH warning panels, found ${warningClasses.length}.`)
}

for (const [index, classes] of warningClasses.entries()) {
  const missing = requiredClasses.filter(className => !classes.includes(className))
  if (missing.length > 0) {
    failures.push(`${relativePath}: SSH warning panel ${index + 1} is missing ${missing.join(', ')}.`)
  }
}

if (failures.length > 0) {
  console.error('Cluster theme check failed:')
  for (const failure of failures) console.error(`- ${failure}`)
  process.exit(1)
}

console.log('Cluster theme check passed.')
