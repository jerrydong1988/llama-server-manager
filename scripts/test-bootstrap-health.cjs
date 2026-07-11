const assert = require('assert')
const fs = require('fs')
const path = require('path')
const ts = require('typescript')

require.extensions['.ts'] = function loadTs(module, filename) {
  const source = fs.readFileSync(filename, 'utf8')
  const output = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2020,
      esModuleInterop: true,
    },
    fileName: filename,
  }).outputText
  module._compile(output, filename)
}

const { resolveHydratedHealth } = require(path.join('..', 'src', 'store', 'bootstrapHealth.ts'))

const existingInstances = [
  { id: 'running-ok', status: 'running', healthCheck: 'ok' },
  { id: 'running-fail', status: 'running', healthCheck: 'fail' },
  { id: 'stopped-old', status: 'stopped', healthCheck: 'fail' },
]

assert.strictEqual(resolveHydratedHealth('running-ok', 'running', existingInstances), 'ok')
assert.strictEqual(resolveHydratedHealth('running-fail', 'running', existingInstances), 'fail')
assert.strictEqual(resolveHydratedHealth('new-running', 'running', existingInstances), 'pending')
assert.strictEqual(resolveHydratedHealth('stopped-old', 'stopped', existingInstances), 'pending')

console.log('bootstrap health hydration regression passed')
