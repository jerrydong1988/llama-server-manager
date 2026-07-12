import fs from 'node:fs'
import path from 'node:path'

const relativePath = 'src/components/DownloadManager.tsx'
const source = fs.readFileSync(path.join(process.cwd(), relativePath), 'utf8')
const browseButtonPattern = /<button\s+onClick=\{handleBrowse\}\s+disabled=\{browsing\}\s+className="([^"]+)"\s*>[\s\S]*?\{t\.modelRepo\.browseFiles\}[\s\S]*?<\/button>/
const match = source.match(browseButtonPattern)
const failures = []

const requiredClasses = [
  'bg-blue-600',
  'text-white',
  'hover:bg-blue-700',
]

if (!match) {
  failures.push(`${relativePath}: browse files button was not found.`)
} else {
  const classes = match[1].split(/\s+/)
  const missing = requiredClasses.filter(className => !classes.includes(className))
  if (missing.length > 0) {
    failures.push(`${relativePath}: browse files button is missing ${missing.join(', ')}.`)
  }
}

if (failures.length > 0) {
  console.error('Download theme check failed:')
  for (const failure of failures) console.error(`- ${failure}`)
  process.exit(1)
}

console.log('Download theme check passed.')
