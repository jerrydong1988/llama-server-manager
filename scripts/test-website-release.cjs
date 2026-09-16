const assert = require('node:assert/strict')
const { test } = require('node:test')
const { parseReleaseNotes, compareVersions, assetUrl, versionParts } = require('./website-release-contract.cjs')

const notes = '# v2.10.0 — Update\n\nReviewed summary.\n\n## Changes\n\n- First change\n\n[完整代码差异](https://github.com/jerrydong1988/llama-server-manager/compare/v2.9.47...v2.10.0)'
test('extracts curated release content and compares numeric versions', () => {
  assert.deepEqual(parseReleaseNotes(notes, '2.10.0'), { title: 'Update', summary: 'Reviewed summary.', previousVersion: '2.9.47', sections: [{ title: 'Changes', items: ['First change'] }] })
  assert.equal(compareVersions('2.10.0', '2.9.47'), 1)
  assert.equal(compareVersions('2.9.47', '2.9.47'), 0)
})
test('rejects prereleases, malformed content, unsupported nested lists and missing release history', () => {
  for (const version of ['v2.9.47', '2.9.48-rc.1', '../foo', '02.9.47']) assert.throws(() => versionParts(version))
  assert.throws(() => parseReleaseNotes(notes, '2.10.1'))
  assert.throws(() => parseReleaseNotes(notes.replace('- First', '  - First'), '2.10.0'))
  assert.throws(() => parseReleaseNotes(notes.replace('v2.9.47...', 'v2.10.1...'), '2.10.0'))
  assert.throws(() => parseReleaseNotes(notes.split('[完整')[0], '2.10.0'))
})
test('installer URLs cannot escape the version or public download host', () => {
  assert.equal(assetUrl('linux-x64', '2.10.0', 'LlamaServerManager_2.10.0_amd64.deb'), 'https://updates.cnzone.net/downloads/v2.10.0/LlamaServerManager_2.10.0_amd64.deb')
  for (const name of ['../secret', 'LlamaServerManager_2.9.47_amd64.deb', 'https://evil.invalid/a.deb']) assert.throws(() => assetUrl('linux-x64', '2.10.0', name))
})
