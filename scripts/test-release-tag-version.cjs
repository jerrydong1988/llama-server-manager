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

const buildWorkflow = fs.readFileSync(
  path.join(__dirname, '..', '.github', 'workflows', 'build.yml'),
  'utf8',
)
const publishWorkflow = fs.readFileSync(
  path.join(__dirname, '..', '.github', 'workflows', 'publish-release.yml'),
  'utf8',
)
assert.match(
  publishWorkflow,
  /workflow_run:\s*[\s\S]*workflows:\s*\[Build\]/,
  'protected publication must descend from a completed Build workflow',
)
assert.match(
  publishWorkflow,
  /stage:\s*[\s\S]*needs:\s*\[qualify, rebuild-windows, rebuild-macos, rebuild-linux\]/,
  'release staging must wait for every exact-source package rebuild',
)
assert.match(
  publishWorkflow,
  /publish:\s*[\s\S]*needs:\s*\[qualify, stage\]/,
  'publication must wait for qualified, immutable staged inputs',
)
assert.match(
  publishWorkflow,
  /releases\/generate-notes/,
  'an empty release body must use GitHub generated release notes',
)
assert.match(
  publishWorkflow,
  /if ! gh release view[\s\S]*gh release create[\s\S]*--notes-file release-stage\/updater-notes\.md/,
  'generated notes must only create a missing release so curated notes remain untouched',
)
assert.doesNotMatch(publishWorkflow, /gh release edit|--clobber/, 'published release metadata and assets must not be replaced')
assert.match(buildWorkflow, /branches:\s*\[master\]/, 'release source Build must protect the default branch')

console.log('release tag version regression passed')
