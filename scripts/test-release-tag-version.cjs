const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const packageJson = require('../package.json')

function runForTag(tag) {
  return spawnSync(process.execPath, ['scripts/check-guide.cjs'], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      GITHUB_REF: `refs/tags/${tag}`,
      GITHUB_REF_NAME: tag,
      GITHUB_REF_TYPE: 'tag',
    },
    encoding: 'utf8',
  })
}

const mismatched = runForTag('v9.9.9')
assert.notEqual(mismatched.status, 0, 'a mismatched release tag must fail validation')
assert.match(`${mismatched.stdout}\n${mismatched.stderr}`, /must match release version/)

const matching = runForTag(`v${packageJson.version}`)
assert.equal(matching.status, 0, matching.stderr || matching.stdout)

console.log('release tag version regression passed')
