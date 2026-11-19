const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const { rustsecWorkflow } = require('./lib/rustsec-workflow.cjs')
const workflow = fs.readFileSync('.github/workflows/build.yml', 'utf8')

test('Rust audit keeps the exact reviewed exception and a locked tool version', () => {
  assert.deepEqual(rustsecWorkflow(workflow).advisories, ['RUSTSEC-2024-0429'])
  assert.deepEqual(rustsecWorkflow(workflow.replace(/\r\n/g, '\n')).advisories, ['RUSTSEC-2024-0429'])
})

test('Rust audit policy rejects missing, weakened or conditional gates', () => {
  const audit = 'cargo audit --file src-tauri/Cargo.lock --ignore RUSTSEC-2024-0429'
  for (const replacement of [
    'echo audit passed', `${audit} || true`, `${audit}; exit 0`,
    `${audit} --ignore RUSTSEC-2026-0001`, `${audit} --ignore RUSTSEC-2024-0429`,
    `${audit} --no-fetch`, `${audit} --no-yanked`, `${audit} --target-os windows`,
    audit.replace('src-tauri/Cargo.lock', 'fixture/Cargo.lock'),
    audit.replace(' --ignore RUSTSEC-2024-0429', ''),
  ]) assert.throws(() => rustsecWorkflow(workflow.replace(audit, replacement)), replacement)
  for (const line of ['if: false', 'continue-on-error: true']) {
    assert.throws(() => rustsecWorkflow(workflow.replace('      - name: Audit Rust dependencies', `      - name: Audit Rust dependencies\n        ${line}`)))
  }
  assert.throws(() => rustsecWorkflow(workflow.replace('--version 0.22.2 --locked', '--version 0.23.0')))
  assert.throws(() => rustsecWorkflow(workflow.replace('  quality:', '  omitted-quality:')))
})
