const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const packageJson = require('../package.json')
const fs = require('node:fs')
const path = require('node:path')

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

const workflow = fs.readFileSync(
  path.join(__dirname, '..', '.github', 'workflows', 'build.yml'),
  'utf8',
)
assert.match(workflow, /finalize-release:/, 'release workflow must have a final notes check')
assert.match(
  workflow,
  /needs:\s*\[build-windows, build-macos, build-linux, build-linux-arm64\]/,
  'release notes must be finalized only after every package job succeeds',
)
assert.match(
  workflow,
  /releases\/generate-notes/,
  'an empty release body must use GitHub generated release notes',
)
assert.match(
  workflow,
  /Release notes already exist; preserving the curated content/,
  'curated release notes must not be overwritten',
)

console.log('release tag version regression passed')
