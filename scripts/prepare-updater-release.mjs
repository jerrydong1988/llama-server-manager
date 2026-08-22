import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import crypto from 'node:crypto'

const root = path.resolve(import.meta.dirname, '..')
const packageJson = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))
const version = process.env.RELEASE_VERSION || packageJson.version
const tag = process.env.RELEASE_TAG || process.env.GITHUB_REF_NAME || `v${version}`
const mode = process.argv[2]
const inputRoot = path.resolve(process.env.UPDATER_INPUT_DIR || path.join(root, 'updater-input'))
const outputRoot = path.resolve(process.env.UPDATER_OUTPUT_DIR || path.join(root, 'updater-publish'))
const releaseRoot = path.join(outputRoot, 'releases', `v${version}`)
const releaseMapPath = path.join(outputRoot, '.release-map.json')
const sourceSha = (process.env.SOURCE_SHA || '').toLowerCase()

if (tag !== `v${version}`) {
  throw new Error(`release tag ${tag} must match package version v${version}`)
}

function filesIn(directory) {
  if (!fs.existsSync(directory)) return []
  return fs.readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile())
    .map(entry => path.join(directory, entry.name))
}

function requireSingleFile(directory, predicate, label) {
  const matches = filesIn(directory).filter(file => predicate(path.basename(file)))
  if (matches.length !== 1) {
    throw new Error(`${label} must contain exactly one updater payload, found ${matches.length}`)
  }
  return matches[0]
}

function readMacPlatform(directory) {
  const marker = path.join(directory, 'platform.txt')
  const platform = fs.readFileSync(marker, 'utf8').trim()
  if (!['darwin-aarch64', 'darwin-x86_64'].includes(platform)) {
    throw new Error(`unsupported macOS updater platform marker: ${platform || '(empty)'}`)
  }
  return platform
}

function canonicalFileName(platform, sourceName) {
  const unsignedSuffix = platform.startsWith('windows-') && sourceName.includes('-unsigned')
    ? '-unsigned'
    : ''
  if (platform === 'windows-x86_64-nsis') {
    return `LlamaServerManager_${version}_${platform}${unsignedSuffix}-setup.exe`
  }
  if (platform === 'windows-x86_64-msi') {
    return `LlamaServerManager_${version}_${platform}${unsignedSuffix}.msi`
  }
  if (platform.startsWith('darwin-')) {
    return `LlamaServerManager_${version}_${platform}.app.tar.gz`
  }
  throw new Error(`unsupported updater platform: ${platform}`)
}

function stageRelease() {
  const macDirectory = path.join(inputRoot, 'macos')
  const macPlatform = readMacPlatform(macDirectory)
  const sources = [
    {
      platform: 'windows-x86_64-nsis',
      file: requireSingleFile(
        path.join(inputRoot, 'windows'),
        name => name.toLowerCase().endsWith('.exe'),
        'Windows updater input',
      ),
    },
    {
      platform: 'windows-x86_64-msi',
      file: requireSingleFile(
        path.join(inputRoot, 'windows'),
        name => name.toLowerCase().endsWith('.msi'),
        'Windows MSI updater input',
      ),
    },
    {
      platform: macPlatform,
      file: requireSingleFile(
        macDirectory,
        name => name.endsWith('.app.tar.gz'),
        'macOS updater input',
      ),
    },
  ]

  fs.rmSync(outputRoot, { recursive: true, force: true })
  fs.mkdirSync(releaseRoot, { recursive: true })

  const releaseMap = {}
  for (const source of sources) {
    if (releaseMap[source.platform]) {
      throw new Error(`duplicate updater platform: ${source.platform}`)
    }
    const fileName = canonicalFileName(source.platform, path.basename(source.file))
    fs.copyFileSync(source.file, path.join(releaseRoot, fileName))
    releaseMap[source.platform] = fileName
  }
  fs.writeFileSync(releaseMapPath, `${JSON.stringify(releaseMap, null, 2)}\n`, 'utf8')
  console.log(`Prepared ${Object.keys(releaseMap).length} updater payloads for v${version}.`)
}

function createManifest() {
  const publicBaseUrl = (process.env.R2_PUBLIC_BASE_URL || '').replace(/\/+$/, '')
  if (!publicBaseUrl.startsWith('https://')) {
    throw new Error('R2_PUBLIC_BASE_URL must be an HTTPS origin')
  }

  const releaseMap = JSON.parse(fs.readFileSync(releaseMapPath, 'utf8'))
  if (!/^[0-9a-f]{40}$/.test(sourceSha)) {
    throw new Error('SOURCE_SHA must be the exact 40-character release commit SHA')
  }
  const versionParts = version.split('.').map(part => Number(part))
  if (
    versionParts.length !== 3
    || versionParts.some(part => !Number.isSafeInteger(part) || part < 0 || part > 999_999)
  ) {
    throw new Error(`package version ${version} is outside the monotonic release-counter contract`)
  }
  const releaseCounter = versionParts[0] * 1_000_000_000_000
    + versionParts[1] * 1_000_000
    + versionParts[2]
  if (!Number.isSafeInteger(releaseCounter) || releaseCounter <= 0) {
    throw new Error(`package version ${version} has no valid monotonic release counter`)
  }
  const platforms = {}
  for (const [platform, fileName] of Object.entries(releaseMap)) {
    const payloadPath = path.join(releaseRoot, fileName)
    const signaturePath = `${payloadPath}.sig`
    if (!fs.existsSync(payloadPath) || !fs.existsSync(signaturePath)) {
      throw new Error(`missing signed updater payload for ${platform}`)
    }
    const signature = fs.readFileSync(signaturePath, 'utf8').trim()
    if (!signature) throw new Error(`empty updater signature for ${platform}`)
    platforms[platform] = {
      signature,
      url: `${publicBaseUrl}/releases/v${version}/${encodeURIComponent(fileName)}`,
      sha256: crypto.createHash('sha256').update(fs.readFileSync(payloadPath)).digest('hex'),
    }
  }

  const notesPath = process.env.UPDATER_NOTES_FILE
  const notes = notesPath && fs.existsSync(notesPath)
    ? fs.readFileSync(notesPath, 'utf8').trim()
    : ''
  const manifest = {
    version,
    notes,
    pub_date: new Date().toISOString(),
    release_tag: tag,
    source_sha: sourceSha,
    release_counter: releaseCounter,
    platforms,
  }
  fs.writeFileSync(path.join(outputRoot, 'release-envelope.json'), `${JSON.stringify(manifest)}\n`, 'utf8')
  console.log(`Created release-envelope.json for ${Object.keys(platforms).join(', ')}.`)
}

if (mode === '--stage') stageRelease()
else if (mode === '--manifest') createManifest()
else throw new Error('usage: node scripts/prepare-updater-release.mjs --stage|--manifest')
