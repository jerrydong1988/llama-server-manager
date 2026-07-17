const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const baselinePath = path.join(__dirname, 'llama-parameter-baseline.json')
const arguments = new Set(process.argv.slice(2))
const writeMode = arguments.has('--write')
const softMode = arguments.has('--soft')
const reportArgument = process.argv.indexOf('--report')
const reportPath = reportArgument >= 0 ? path.resolve(process.argv[reportArgument + 1]) : null
const repository = 'ggerganov/llama.cpp'

const headers = {
  Accept: 'application/vnd.github+json',
  'User-Agent': 'llama-server-manager-upstream-watch',
  'X-GitHub-Api-Version': '2022-11-28',
}
if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`

const sha256 = value => crypto.createHash('sha256').update(value).digest('hex')

async function fetchText(url) {
  const response = await fetch(url, { headers })
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}: ${url}`)
  return response.text()
}

async function fetchJson(apiPath) {
  return JSON.parse(await fetchText(`https://api.github.com/repos/${repository}${apiPath}`))
}

async function resolveTagCommit(tag) {
  const reference = await fetchJson(`/git/ref/tags/${encodeURIComponent(tag)}`)
  if (reference.object.type === 'commit') return reference.object.sha
  const annotatedTag = await fetchJson(`/git/tags/${reference.object.sha}`)
  return annotatedTag.object.sha
}

function splitMarkdownRow(line) {
  const cells = []
  let current = ''
  let inCode = false
  let escaped = false
  for (const character of line.trim()) {
    if (escaped) {
      current += character
      escaped = false
    } else if (character === '\\') {
      current += character
      escaped = true
    } else if (character === '`') {
      current += character
      inCode = !inCode
    } else if (character === '|' && !inCode) {
      cells.push(current.trim())
      current = ''
    } else {
      current += character
    }
  }
  cells.push(current.trim())
  return cells
}

function extractFlags(value) {
  const flags = new Set()
  for (let index = 0; index < value.length; index += 1) {
    if (value[index] !== '-') continue
    const previous = value[index - 1] || ' '
    if (/[a-z0-9_]/i.test(previous)) continue
    let end = index + 1
    while (end < value.length && /[a-z0-9-]/i.test(value[end])) end += 1
    const token = value.slice(index, end)
    const body = token.replace(/^-+/, '')
    if (body && /[a-z]/i.test(body)) flags.add(token)
    index = Math.max(index, end - 1)
  }
  return [...flags]
}

function normalizeText(value) {
  return value.replace(/\s+/g, ' ').trim()
}

function parseParameterTable(markdown) {
  const parameters = new Map()
  for (const line of markdown.split(/\r?\n/)) {
    if (!line.trimStart().startsWith('|')) continue
    const cells = splitMarkdownRow(line)
    if (cells.length < 4) continue
    const synopsis = normalizeText(cells[1] || '')
    const description = normalizeText(cells[2] || '')
    const aliases = extractFlags(synopsis)
    if (aliases.length === 0) continue
    const canonical = aliases.find(flag => flag.startsWith('--')) || aliases[0]
    parameters.set(canonical, {
      canonical,
      aliases: [...new Set(aliases)].sort(),
      synopsisHash: sha256(synopsis),
      descriptionHash: sha256(description),
    })
  }
  const entries = [...parameters.values()].sort((left, right) => left.canonical.localeCompare(right.canonical))
  if (entries.length < 100) {
    throw new Error(`only ${entries.length} llama-server parameter rows were parsed`)
  }
  return {
    parameterCount: entries.length,
    snapshotHash: sha256(JSON.stringify(entries)),
    parameters: entries,
  }
}

async function readSnapshot(ref, commit) {
  const readme = await fetchText(`https://raw.githubusercontent.com/${repository}/${encodeURIComponent(commit)}/tools/server/README.md`)
  return { ref, commit, ...parseParameterTable(readme) }
}

function compareSnapshots(baseline, current) {
  if (!baseline) {
    return { added: current.parameters.map(item => item.canonical), removed: [], aliasesChanged: [], synopsisChanged: [], descriptionChanged: [] }
  }
  const before = new Map(baseline.parameters.map(item => [item.canonical, item]))
  const after = new Map(current.parameters.map(item => [item.canonical, item]))
  const added = [...after.keys()].filter(flag => !before.has(flag)).sort()
  const removed = [...before.keys()].filter(flag => !after.has(flag)).sort()
  const shared = [...after.keys()].filter(flag => before.has(flag))
  return {
    added,
    removed,
    aliasesChanged: shared.filter(flag => JSON.stringify(before.get(flag).aliases) !== JSON.stringify(after.get(flag).aliases)).sort(),
    synopsisChanged: shared.filter(flag => before.get(flag).synopsisHash !== after.get(flag).synopsisHash).sort(),
    descriptionChanged: shared.filter(flag => before.get(flag).descriptionHash !== after.get(flag).descriptionHash).sort(),
  }
}

const hasChanges = diff => Object.values(diff).some(items => items.length > 0)
const formatFlags = flags => flags.length > 0 ? flags.map(flag => `\`${flag}\``).join(', ') : 'None'

function snapshotReport(title, baseline, current, diff) {
  return [
    `## ${title}`,
    '',
    `- Baseline: ${baseline ? `${baseline.ref} (${baseline.commit})` : 'missing'}`,
    `- Current: ${current.ref} (${current.commit})`,
    `- Parameter rows: ${baseline?.parameterCount ?? 0} -> ${current.parameterCount}`,
    `- Added: ${formatFlags(diff.added)}`,
    `- Removed: ${formatFlags(diff.removed)}`,
    `- Alias changes: ${formatFlags(diff.aliasesChanged)}`,
    `- Synopsis changes: ${formatFlags(diff.synopsisChanged)}`,
    `- Description/default changes: ${formatFlags(diff.descriptionChanged)}`,
    '',
  ].join('\n')
}

function writeOutput(name, value) {
  if (!process.env.GITHUB_OUTPUT) return
  fs.appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${value}\n`, 'utf8')
}

async function main() {
  const baseline = JSON.parse(fs.readFileSync(baselinePath, 'utf8'))
  const latestRelease = await fetchJson('/releases/latest')
  const releaseCommit = await resolveTagCommit(latestRelease.tag_name)
  const masterCommit = (await fetchJson('/commits/master')).sha
  const [releaseSnapshot, masterSnapshot] = await Promise.all([
    readSnapshot(latestRelease.tag_name, releaseCommit),
    readSnapshot('master', masterCommit),
  ])

  if (writeMode) {
    const stableAliases = new Set(releaseSnapshot.parameters.flatMap(parameter => parameter.aliases))
    const next = {
      schemaVersion: 2,
      verifiedAt: new Date().toISOString().slice(0, 10),
      source: `https://github.com/${repository}`,
      supportPolicy: {
        authoritative: 'latest-stable-release',
        compatibilityWindow: 'latest stable plus two previous stable releases through runtime capability negotiation',
        master: 'canary-only',
      },
      upstreamRelease: latestRelease.tag_name,
      upstreamCommit: releaseCommit,
      upstreamMasterChecked: masterCommit,
      releaseSnapshot,
      masterSnapshot,
      masterPreviewFlags: (baseline.masterPreviewFlags || []).filter(flag => !stableAliases.has(flag)),
      structuredFlagsAdded: baseline.structuredFlagsAdded || [],
      defaultTrueNegativeFlags: baseline.defaultTrueNegativeFlags || [],
      intentionalCustomArgsOnly: baseline.intentionalCustomArgsOnly || {},
    }
    fs.writeFileSync(baselinePath, `${JSON.stringify(next, null, 2)}\n`, 'utf8')
    console.log(`Updated llama.cpp baseline to ${latestRelease.tag_name} (${releaseSnapshot.parameterCount} parameters).`)
    return
  }

  const releaseDiff = compareSnapshots(baseline.releaseSnapshot, releaseSnapshot)
  const masterDiff = compareSnapshots(baseline.masterSnapshot, masterSnapshot)
  const releaseDrift = baseline.upstreamRelease !== latestRelease.tag_name || hasChanges(releaseDiff)
  const masterDrift = hasChanges(masterDiff)
  const report = [
    '# llama.cpp Parameter Drift Report',
    '',
    `Generated: ${new Date().toISOString()}`,
    '',
    snapshotReport('Latest stable release (authoritative)', baseline.releaseSnapshot, releaseSnapshot, releaseDiff),
    snapshotReport('Master branch (canary)', baseline.masterSnapshot, masterSnapshot, masterDiff),
    'Stable-release drift blocks the compatibility gate. Master-only drift opens a tracking issue but remains a canary warning.',
    '',
  ].join('\n')
  if (reportPath) fs.writeFileSync(reportPath, report, 'utf8')
  console.log(report)
  writeOutput('release_drift', releaseDrift)
  writeOutput('master_drift', masterDrift)
  writeOutput('error', false)
  if (releaseDrift && !softMode) process.exitCode = 1
}

main().catch(error => {
  const report = `# llama.cpp Parameter Drift Report\n\nUpstream check failed: ${error.message}\n`
  if (reportPath) fs.writeFileSync(reportPath, report, 'utf8')
  console.error(report)
  writeOutput('release_drift', false)
  writeOutput('master_drift', false)
  writeOutput('error', true)
  if (!softMode) process.exitCode = 1
})
