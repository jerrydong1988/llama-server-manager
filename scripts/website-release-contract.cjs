// Shared wire contract. Keep the website repository's copy byte-identical.
const assert = require('node:assert/strict')

const REPOSITORY = 'jerrydong1988/llama-server-manager'
const DOWNLOAD_ORIGIN = 'https://updates.cnzone.net'
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/
const ASSET_IDS = ['windows-exe', 'windows-msi', 'macos-dmg', 'linux-x64', 'linux-arm64']

function versionParts(version) {
  assert.equal(typeof version, 'string', 'version must be a string')
  assert.match(version, VERSION, 'only stable semantic versions are supported')
  const parts = version.split('.').map(Number)
  assert.ok(parts.every(Number.isSafeInteger), 'version component too large')
  return parts
}

function compareVersions(a, b) {
  const left = versionParts(a), right = versionParts(b)
  for (let i = 0; i < 3; i++) if (left[i] !== right[i]) return Math.sign(left[i] - right[i])
  return 0
}

function text(value, label, max = 2000) {
  assert.equal(typeof value, 'string', `${label} must be text`)
  assert.ok(value.trim() && value.length <= max && !/[\x00-\x08\x0b\x0c\x0e-\x1f]/.test(value), `invalid ${label}`)
}

function assetPattern(id, version) {
  versionParts(version)
  const escaped = version.replaceAll('.', '\\.')
  const suffix = {
    'windows-exe': 'windows-x86_64-nsis-(?:un)?signed-setup\\.exe',
    'windows-msi': 'windows-x86_64-msi-(?:un)?signed\\.msi',
    'macos-dmg': 'aarch64-(?:adhoc|signed)\\.dmg',
    'linux-x64': 'amd64\\.deb',
    'linux-arm64': 'arm64\\.deb',
  }[id]
  assert.ok(suffix, `unknown asset ${id}`)
  return new RegExp(`^LlamaServerManager_${escaped}_${suffix}$`)
}

function assetUrl(id, version, name) {
  assert.match(name, assetPattern(id, version), `unexpected filename for ${id}`)
  return `${DOWNLOAD_ORIGIN}/${id.startsWith('windows-') ? 'releases' : 'downloads'}/v${version}/${name}`
}

function validateManifest(data) {
  assert.equal(data.schemaVersion, 1, 'unsupported website manifest schema')
  versionParts(data.version)
  assert.ok(compareVersions(data.previousVersion, data.version) < 0, 'previous version must be older')
  assert.match(data.sourceCommit, /^[a-f0-9]{40}$/, 'invalid source commit')
  assert.ok(Number.isSafeInteger(data.releaseId) && data.releaseId > 0, 'invalid release id')
  assert.match(data.releaseDate, /^\d{4}-\d{2}-\d{2}$/, 'invalid release date')
  assert.equal(new Date(`${data.releaseDate}T00:00:00Z`).toISOString().slice(0, 10), data.releaseDate)
  text(data.title, 'title', 160)
  text(data.summary, 'summary', 1000)
  assert.ok(Array.isArray(data.sections) && data.sections.length > 0 && data.sections.length <= 30, 'missing release sections')
  for (const section of data.sections) {
    text(section.title, 'section title', 160)
    assert.ok(Array.isArray(section.items) && section.items.length > 0 && section.items.length <= 100, 'missing section items')
    section.items.forEach(item => text(item, 'release item'))
  }
  assert.ok(Array.isArray(data.assets) && data.assets.length === ASSET_IDS.length, 'expected five installer assets')
  assert.deepEqual([...data.assets.map(a => a.id)].sort(), [...ASSET_IDS].sort(), 'invalid or duplicate asset ids')
  for (const asset of data.assets) {
    assert.equal(asset.url, assetUrl(asset.id, data.version, asset.fileName), 'download URL must match release and platform')
    assert.ok(Number.isSafeInteger(asset.bytes) && asset.bytes > 0, 'invalid asset byte count')
    assert.match(asset.sha256, /^[a-f0-9]{64}$/, 'invalid asset SHA-256')
  }
  return data
}

function parseReleaseNotes(markdown, version) {
  versionParts(version)
  const lines = markdown.replaceAll('\r\n', '\n').trim().split('\n')
  const prefix = `# v${version} — `
  assert.ok(lines[0].startsWith(prefix), `release notes must start with ${prefix}<title>`)
  const title = lines.shift().slice(prefix.length).trim()
  const summary = [], sections = []
  let current = null, previousVersion = null
  for (const raw of lines) {
    const line = raw.trim()
    if (!line) continue
    const comparison = line.match(/^\[完整代码差异\]\(https:\/\/github\.com\/jerrydong1988\/llama-server-manager\/compare\/v([\d.]+)\.\.\.v([\d.]+)\)$/)
    if (comparison) {
      assert.equal(comparison[2], version, 'comparison target differs from release')
      assert.equal(previousVersion, null, 'duplicate comparison link')
      previousVersion = comparison[1]
    } else if (line.startsWith('## ')) {
      current = { title: line.slice(3), items: [] }
      sections.push(current)
    } else if (line.startsWith('- ') && current && raw === line) {
      current.items.push(line.slice(2))
    } else if (!current && !line.startsWith('#')) {
      summary.push(line)
    } else {
      throw new Error(`Unsupported release-note structure: ${line.slice(0, 100)}`)
    }
  }
  text(title, 'title', 160)
  text(summary.join(' '), 'summary', 1000)
  assert.ok(previousVersion && compareVersions(previousVersion, version) < 0, 'missing valid previous-version comparison')
  assert.ok(sections.length && sections.every(s => s.items.length), 'release notes require nonempty sections')
  return { title, summary: summary.join(' '), sections, previousVersion }
}

module.exports = { REPOSITORY, DOWNLOAD_ORIGIN, ASSET_IDS, versionParts, compareVersions, assetPattern, assetUrl, validateManifest, parseReleaseNotes }
