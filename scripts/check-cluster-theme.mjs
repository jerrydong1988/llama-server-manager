import fs from 'node:fs'
import path from 'node:path'

const relativePath = 'src/components/ClusterPage/ClusterPage.tsx'
const source = fs.readFileSync(path.join(process.cwd(), relativePath), 'utf8')
const securityPanelPattern = /<div className="([^"]+)">\s*\{t\.clusterPage\.agentSecurityNote\}\s*<\/div>/g
const securityPanels = [...source.matchAll(securityPanelPattern)].map(match => match[1].split(/\s+/))
const failures = []

const requiredClasses = [
  'border-violet-500/20',
  'bg-violet-500/10',
  'text-violet-200',
]

if (securityPanels.length !== 1) {
  failures.push(`${relativePath}: expected 1 Secure Agent security panel, found ${securityPanels.length}.`)
}

for (const classes of securityPanels) {
  const missing = requiredClasses.filter(className => !classes.includes(className))
  if (missing.length > 0) {
    failures.push(`${relativePath}: Secure Agent security panel is missing ${missing.join(', ')}.`)
  }
}

if (!source.includes('data-guide="cluster-agent"')) {
  failures.push(`${relativePath}: Secure Agent enrollment action must remain available to the guide.`)
}

if (failures.length > 0) {
  console.error('Cluster theme check failed:')
  for (const failure of failures) console.error(`- ${failure}`)
  process.exit(1)
}

console.log('Cluster theme check passed.')
