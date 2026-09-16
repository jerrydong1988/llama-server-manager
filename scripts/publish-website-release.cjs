const assert = require('node:assert/strict')
const { createHash } = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')
const { REPOSITORY, DOWNLOAD_ORIGIN, ASSET_IDS, versionParts, assetPattern, assetUrl, validateManifest, parseReleaseNotes } = require('./website-release-contract.cjs')

async function github(endpoint) {
  const response = await fetch(`https://api.github.com/repos/${REPOSITORY}/${endpoint}`, {
    headers: { Accept: 'application/vnd.github+json', ...(process.env.GH_TOKEN ? { Authorization: `Bearer ${process.env.GH_TOKEN}` } : {}) },
    redirect: 'error', signal: AbortSignal.timeout(30_000),
  })
  assert.equal(response.status, 200, `GitHub ${endpoint}: HTTP ${response.status}`)
  return response.json()
}

async function fileDigest(url) {
  // Public artifacts deliberately receive no GitHub or Cloudflare credentials.
  const response = await fetch(url, { signal: AbortSignal.timeout(180_000) })
  assert.equal(response.status, 200, `Download HTTP ${response.status}: ${url}`)
  assert.doesNotMatch(response.headers.get('content-type') || '', /text\/html/, 'download returned HTML')
  const hash = createHash('sha256')
  let bytes = 0
  for await (const chunk of response.body) { hash.update(chunk); bytes += chunk.length }
  return { bytes, sha256: hash.digest('hex') }
}

async function generate(tag) {
  assert.ok(tag.startsWith('v'), 'expected vX.Y.Z tag')
  const version = tag.slice(1)
  versionParts(version)
  const release = await github(`releases/tags/${tag}`)
  assert.equal(release.tag_name, tag)
  assert.equal(release.draft, false, 'draft releases cannot update the website')
  assert.equal(release.prerelease, false, 'prereleases cannot update the website')
  assert.equal((await github('releases/latest')).tag_name, tag, 'only latest stable release may update the website')
  const commit = await github(`commits/${tag}`)
  const notesFile = await github(`contents/docs/release-notes/${tag}.md?ref=${commit.sha}`)
  assert.equal(notesFile.encoding, 'base64')
  const notes = parseReleaseNotes(Buffer.from(notesFile.content, 'base64').toString('utf8'), version)
  const assets = []
  for (const id of ASSET_IDS) {
    const matches = release.assets.filter(a => assetPattern(id, version).test(a.name))
    assert.equal(matches.length, 1, `expected exactly one ${id} installer`)
    const asset = matches[0]
    const url = assetUrl(id, version, asset.name)
    const githubUrl = `https://github.com/${REPOSITORY}/releases/download/${tag}/${asset.name}`
    const [publicFile, githubFile] = await Promise.all([fileDigest(url), fileDigest(githubUrl)])
    assert.deepEqual(publicFile, githubFile, `${id}: public download differs from GitHub`)
    assert.equal(publicFile.bytes, asset.size, `${id}: asset size differs`)
    if (asset.digest) assert.equal(asset.digest, `sha256:${publicFile.sha256}`, `${id}: GitHub digest differs`)
    assets.push({ id, fileName: asset.name, ...publicFile, url })
    console.log(`Verified ${id}: ${publicFile.bytes} bytes, ${publicFile.sha256}`)
  }
  const updater = await fetch(`${DOWNLOAD_ORIGIN}/latest.json`, { cache: 'no-store', signal: AbortSignal.timeout(30_000) })
  assert.equal(updater.status, 200, 'updater manifest must be publicly readable')
  assert.equal((await updater.json()).version, version, 'updater publication has not completed')
  return validateManifest({ schemaVersion: 1, version, releaseId: release.id, sourceCommit: commit.sha,
    releaseDate: new Intl.DateTimeFormat('en-CA', { timeZone: 'Asia/Shanghai', year: 'numeric', month: '2-digit', day: '2-digit' }).format(new Date(release.published_at)),
    ...notes, assets })
}

async function main() {
  if (process.argv[2] === '--check-notes') {
    const version = require('../package.json').version
    parseReleaseNotes(fs.readFileSync(path.join(__dirname, `../docs/release-notes/v${version}.md`), 'utf8'), version)
    console.log(`Website release notes v${version} validated`)
    return
  }
  const data = await generate(process.argv[2] || '')
  const destination = process.argv[3]
  assert.ok(destination, 'output file is required')
  fs.writeFileSync(destination, `${JSON.stringify(data, null, 2)}\n`, 'utf8')
}

if (require.main === module) main().catch(error => { console.error(error.message); process.exitCode = 1 })
module.exports = { generate, fileDigest }
