const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const policy = JSON.parse(fs.readFileSync(path.join(root, '.github', 'rustsec-allowlist.json'), 'utf8'))
const workflow = fs.readFileSync(path.join(root, '.github', 'workflows', 'build.yml'), 'utf8')
const ignoreLine = workflow.match(/^\s*ignore:\s*(.+)$/m)
if (!ignoreLine) throw new Error('RustSec workflow ignore list is missing')

const workflowIds = new Set(ignoreLine[1].split(',').map(id => id.trim()).filter(Boolean))
const policyIds = new Set(policy.entries.map(entry => entry.id))
const missingFromPolicy = [...workflowIds].filter(id => !policyIds.has(id))
const missingFromWorkflow = [...policyIds].filter(id => !workflowIds.has(id))
if (missingFromPolicy.length || missingFromWorkflow.length) {
  throw new Error(`RustSec allowlist drift: undocumented=${missingFromPolicy.join(',') || '-'} inactive=${missingFromWorkflow.join(',') || '-'}`)
}

const reviewBy = new Date(`${policy.reviewBy}T23:59:59Z`)
if (!Number.isFinite(reviewBy.getTime()) || reviewBy <= new Date()) {
  throw new Error(`RustSec allowlist review expired on ${policy.reviewBy}`)
}

if (policy.entries.some(entry => !entry.package || !entry.kind)) {
  throw new Error('Each RustSec exception must name its package and warning kind')
}

console.log(`RustSec allowlist policy passed (${policy.entries.length} entries, review by ${policy.reviewBy}).`)
