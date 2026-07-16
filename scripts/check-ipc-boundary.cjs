const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const commandsDir = path.join(root, 'src-tauri', 'src', 'commands')
const srcDir = path.join(root, 'src')

function collect(dir, pattern) {
  const files = []
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name)
    if (entry.isDirectory()) files.push(...collect(fullPath, pattern))
    else if (pattern.test(entry.name)) files.push(fullPath)
  }
  return files
}

const commandPattern = /#\[tauri::command\][\s\S]{0,300}?\bpub\s+(?:async\s+)?fn\s+(\w+)\s*\([\s\S]*?\)\s*(?:->\s*([^\{]+))?\s*\{/g
const findings = []
let commandCount = 0
let fallibleCount = 0

for (const file of collect(commandsDir, /\.rs$/)) {
  const source = fs.readFileSync(file, 'utf8')
  for (const match of source.matchAll(commandPattern)) {
    commandCount += 1
    const returnType = (match[2] || '').replace(/\s+/g, ' ').trim()
    if (!returnType.includes('Result<')) continue
    fallibleCount += 1
    if (!returnType.includes('AppResult<')) {
      const line = source.slice(0, match.index).split('\n').length
      findings.push(`${path.relative(root, file)}:${line}: ${match[1]} returns ${returnType}`)
    }
  }
}

const directInvokeImport = /from\s+['"]@tauri-apps\/api\/core['"]/g
for (const file of collect(srcDir, /\.(?:ts|tsx)$/)) {
  if (path.normalize(file) === path.join(srcDir, 'lib', 'ipc.ts')) continue
  const source = fs.readFileSync(file, 'utf8')
  for (const match of source.matchAll(directInvokeImport)) {
    const line = source.slice(0, match.index).split('\n').length
    findings.push(`${path.relative(root, file)}:${line}: direct Tauri core import bypasses src/lib/ipc.ts`)
  }
}

if (findings.length > 0) {
  console.error('IPC error boundary check failed:')
  console.error(findings.join('\n'))
  process.exit(1)
}

console.log(`IPC boundary check passed (${commandCount} commands, ${fallibleCount} structured fallible commands).`)
