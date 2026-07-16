const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const budgets = [
  { file: 'src/App.tsx', maxLines: 500, boundary: 'useCommandCenterModel' },
  { file: 'src/components/DownloadManager.tsx', maxLines: 1250, boundary: 'DownloadPrimitives' },
  { file: 'src/components/ConfigPage.tsx', maxLines: 1000, boundary: 'configWorkspace' },
  { file: 'src/components/InstanceManager.tsx', maxLines: 900, boundary: 'InstanceModelPicker' },
]

const failures = []
for (const budget of budgets) {
  const source = fs.readFileSync(path.join(root, budget.file), 'utf8')
  const lines = source.split(/\r?\n/).length
  if (lines > budget.maxLines) failures.push(`${budget.file}: ${lines} lines exceeds ${budget.maxLines}`)
  if (!source.includes(budget.boundary)) failures.push(`${budget.file}: missing extracted boundary ${budget.boundary}`)
}

if (failures.length) {
  console.error(failures.join('\n'))
  process.exit(1)
}
console.log(`Component complexity budgets passed (${budgets.length} entry points).`)
