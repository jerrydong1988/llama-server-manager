const fs = require('node:fs')

const workflow = fs.readFileSync('.github/workflows/build.yml', 'utf8')
const tauriConfig = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'))
const readme = fs.readFileSync('README.md', 'utf8')
const signingPolicy = fs.readFileSync('CODE_SIGNING_POLICY.md', 'utf8')
const privacyPolicy = fs.readFileSync('PRIVACY.md', 'utf8')
const signingGuide = fs.readFileSync('docs/RELEASE_SIGNING.md', 'utf8')
const failures = []
const rustsecNode24Commit = '858dc40f52ca2b8570b7a997c1c4e35c6fc9a432'
const releaseNode24Commit = '3d0d9888cb7fd7b750713d6e236d1fcb99157228'
const approvedRustsecAdvisories = [
  'RUSTSEC-2024-0370',
  'RUSTSEC-2024-0411',
  'RUSTSEC-2024-0412',
  'RUSTSEC-2024-0413',
  'RUSTSEC-2024-0414',
  'RUSTSEC-2024-0415',
  'RUSTSEC-2024-0416',
  'RUSTSEC-2024-0417',
  'RUSTSEC-2024-0418',
  'RUSTSEC-2024-0419',
  'RUSTSEC-2024-0420',
  'RUSTSEC-2024-0429',
  'RUSTSEC-2025-0075',
  'RUSTSEC-2025-0080',
  'RUSTSEC-2025-0081',
  'RUSTSEC-2025-0098',
  'RUSTSEC-2025-0100',
]

function jobBody(name) {
  const marker = `  ${name}:`
  const start = workflow.indexOf(marker)
  if (start < 0) return ''
  const rest = workflow.slice(start + marker.length)
  const next = rest.search(/^  [a-zA-Z0-9_-]+:/m)
  return next < 0 ? rest : rest.slice(0, next)
}

for (const job of ['build-windows', 'build-macos', 'build-linux', 'build-linux-arm64']) {
  const body = jobBody(job)
  if (!body) {
    failures.push(`missing CI job ${job}`)
    continue
  }
  if (!body.includes('components: clippy')) failures.push(`${job} does not install Clippy`)
  if (!body.includes('cargo test --manifest-path src-tauri/Cargo.toml --locked')) failures.push(`${job} does not run Rust tests`)
  const frontendBuildIndex = body.indexOf('run: npm run build')
  const rustTestIndex = body.indexOf('cargo test --manifest-path src-tauri/Cargo.toml --locked')
  if (frontendBuildIndex < 0 || frontendBuildIndex > rustTestIndex) {
    failures.push(`${job} does not create frontendDist before compiling Rust tests`)
  }
  if (!body.includes('cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings')) {
    failures.push(`${job} does not enforce warning-free Clippy`)
  }
}

const qualityJob = jobBody('quality')
if (!qualityJob.includes(`RustSec/audit-check@${rustsecNode24Commit}`)) {
  failures.push('quality job does not pin the reviewed Node 24 RustSec action commit')
}
if (!qualityJob.includes('working-directory: src-tauri')) {
  failures.push('RustSec audit is not scoped to the Tauri Cargo.lock')
}
const rustsecIgnoreMatch = qualityJob.match(/^\s+ignore:\s*([^\r\n]+)$/m)
const configuredAdvisories = rustsecIgnoreMatch
  ? rustsecIgnoreMatch[1].split(',').map(value => value.trim()).filter(Boolean)
  : []
const uniqueConfiguredAdvisories = [...new Set(configuredAdvisories)].sort()
const expectedAdvisories = [...approvedRustsecAdvisories].sort()
if (JSON.stringify(uniqueConfiguredAdvisories) !== JSON.stringify(expectedAdvisories)) {
  failures.push('RustSec audit exceptions do not exactly match the 17 approved upstream advisories')
}
if (configuredAdvisories.length !== uniqueConfiguredAdvisories.length) {
  failures.push('RustSec audit exceptions contain duplicate advisory IDs')
}
for (const forbidden of [
  'rustsec/audit-check@v2.0.0',
  'ACTIONS_ALLOW_USE_UNSECURE_NODE_VERSION',
  'informational_warnings = []',
]) {
  if (workflow.includes(forbidden)) failures.push(`RustSec workflow contains forbidden broad bypass ${forbidden}`)
}

const releaseUploadMatches = [...workflow.matchAll(/uses:\s*softprops\/action-gh-release@([^\s]+)/g)]
if (releaseUploadMatches.length !== 6) {
  failures.push(`release workflow must contain exactly 6 release upload steps, found ${releaseUploadMatches.length}`)
}
for (const match of releaseUploadMatches) {
  if (match[1] !== releaseNode24Commit) {
    failures.push(`release upload does not pin the reviewed Node 24 action commit: ${match[1]}`)
  }
}
if (workflow.includes('::warning::')) {
  failures.push('expected code-signing fallbacks must not emit warning annotations')
}

const windowsJob = jobBody('build-windows')
if (!windowsJob.includes('actions: read')) failures.push('Windows SignPath job cannot read GitHub Actions build metadata')
for (const token of [
  'Detect SignPath configuration',
  'Upload unsigned Windows installers for signing',
  'Submit Windows installers to SignPath',
  'signpath/github-action-submit-signing-request@v2',
  'SIGNPATH_API_TOKEN',
  'SIGNPATH_ORGANIZATION_ID',
  'SIGNPATH_PROJECT_SLUG',
  'SIGNPATH_SIGNING_POLICY_SLUG',
  'SIGNPATH_ARTIFACT_CONFIGURATION_SLUG',
  'Prepare clearly labeled unsigned Windows release assets',
  '-unsigned$extension',
]) {
  if (!windowsJob.includes(token)) failures.push(`Windows optional SignPath flow is missing ${token}`)
}
for (const token of [
  'WINDOWS_CERTIFICATE',
  'WINDOWS_CERTIFICATE_PASSWORD',
  'WINDOWS_CERTIFICATE_THUMBPRINT',
  'Import-PfxCertificate',
]) {
  if (windowsJob.includes(token)) failures.push(`Windows workflow still contains obsolete PFX signing token ${token}`)
}

const macJob = jobBody('build-macos')
if (!macJob.includes('runs-on: macos-15')) failures.push('macOS runner is not pinned and may migrate without review')
for (const token of [
  'Detect Apple signing configuration',
  'Import Apple signing certificate',
  'Build ad-hoc signed Tauri package',
  'Build signed and notarized Tauri release',
  'APPLE_CERTIFICATE',
  'APPLE_CERTIFICATE_PASSWORD',
  'APPLE_SIGNING_IDENTITY',
  'APPLE_ID',
  'APPLE_PASSWORD',
  'APPLE_TEAM_ID',
  '-adhoc',
]) {
  if (!macJob.includes(token)) failures.push(`macOS optional signing flow is missing ${token}`)
}
if (macJob.includes('Validate macOS signing secrets')) {
  failures.push('macOS workflow still blocks tag releases when Apple signing secrets are unavailable')
}
if (!macJob.includes('building an ad-hoc signed macOS package')) {
  failures.push('macOS workflow does not explain the unsigned release fallback')
}

for (const link of ['PRIVACY.md', 'CODE_SIGNING_POLICY.md', 'docs/RELEASE_SIGNING.md', 'docs/DEPENDENCY_AUDIT.md']) {
  if (!readme.includes(link)) failures.push(`README does not link to ${link}`)
}
if (!signingPolicy.includes('Free code signing provided by SignPath.io, certificate by SignPath Foundation')) {
  failures.push('code signing policy is missing the SignPath Foundation attribution')
}
for (const service of ['GitHub', 'ModelScope', 'Hugging Face']) {
  if (!privacyPolicy.includes(service)) failures.push(`privacy policy does not disclose ${service} network access`)
}
for (const obsoleteToken of ['WINDOWS_CERTIFICATE', 'Import-PfxCertificate']) {
  if (signingGuide.includes(obsoleteToken)) failures.push(`release signing guide still documents obsolete token ${obsoleteToken}`)
}

if (tauriConfig.bundle?.macOS?.signingIdentity !== '-') {
  failures.push('macOS non-release artifacts do not use an ad-hoc signing identity')
}

for (const localConfig of ['configs/instances.json', 'src-tauri/configs/instances.json']) {
  if (fs.existsSync(localConfig)) failures.push(`machine-local config is still tracked: ${localConfig}`)
}

if (failures.length > 0) {
  console.error('Cross-platform release check failed:')
  failures.forEach(failure => console.error(`- ${failure}`))
  process.exit(1)
}

console.log('Cross-platform release check passed.')
