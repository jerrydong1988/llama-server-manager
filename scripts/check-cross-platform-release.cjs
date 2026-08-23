const fs = require('node:fs')

const workflow = fs.readFileSync('.github/workflows/build.yml', 'utf8')
const protectedReleaseWorkflow = fs.readFileSync('.github/workflows/publish-release.yml', 'utf8')
const dependencyReviewWorkflow = fs.readFileSync('.github/workflows/dependency-review.yml', 'utf8')
const tauriConfig = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'))
const updaterBuildConfig = JSON.parse(fs.readFileSync('src-tauri/tauri.updater.conf.json', 'utf8'))
const readme = fs.readFileSync('README.md', 'utf8')
const guide = fs.readFileSync('GUIDE.md', 'utf8')
const signingPolicy = fs.readFileSync('CODE_SIGNING_POLICY.md', 'utf8')
const privacyPolicy = fs.readFileSync('PRIVACY.md', 'utf8')
const signingGuide = fs.readFileSync('docs/RELEASE_SIGNING.md', 'utf8')
const failures = []
const rustsecNode24Commit = '858dc40f52ca2b8570b7a997c1c4e35c6fc9a432'
const reviewedActionPins = new Map([
  ['actions/checkout', new Set(['3d3c42e5aac5ba805825da76410c181273ba90b1', '11d5960a326750d5838078e36cf38b85af677262'])],
  ['actions/dependency-review-action', new Set(['2031cfc080254a8a887f58cffee85186f0e49e48'])],
  ['actions/download-artifact', new Set(['3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c'])],
  ['actions/setup-node', new Set(['249970729cb0ef3589644e2896645e5dc5ba9c38', '49933ea5288caeca8642d1e84afbd3f7d6820020'])],
  ['actions/upload-artifact', new Set(['043fb46d1a93c77aae656e7c1c64a875d1fc6a0a'])],
  ['dtolnay/rust-toolchain', new Set(['4360b52568e2003a75bf9bc1d59f33a8e3fc893c'])],
  ['RustSec/audit-check', new Set([rustsecNode24Commit])],
  ['Swatinem/rust-cache', new Set(['6323deb102c322ba6fcbdcafc7e3dddab59af2b6'])],
])
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

function workflowJobBody(document, name) {
  const marker = `  ${name}:`
  const start = document.indexOf(marker)
  if (start < 0) return ''
  const rest = document.slice(start + marker.length)
  const next = rest.search(/^  [a-zA-Z0-9_-]+:/m)
  return next < 0 ? rest : rest.slice(0, next)
}

function jobBody(name) {
  return workflowJobBody(workflow, name)
}

const workflowDirectory = '.github/workflows'
for (const file of fs.readdirSync(workflowDirectory).filter(name => /\.ya?ml$/i.test(name))) {
  const contents = fs.readFileSync(`${workflowDirectory}/${file}`, 'utf8')
  for (const match of contents.matchAll(/uses:\s*([^\s#]+)@([^\s#]+)/g)) {
    const [, action, reference] = match
    if (!/^[0-9a-f]{40}$/.test(reference)) {
      failures.push(`${file} uses mutable action reference ${action}@${reference}`)
      continue
    }
    const approved = reviewedActionPins.get(action)
    if (!approved?.has(reference)) {
      failures.push(`${file} uses unreviewed action commit ${action}@${reference}`)
    }
  }
}

for (const job of ['build-windows', 'build-macos', 'build-linux', 'build-linux-arm64']) {
  const body = jobBody(job)
  if (!body) {
    failures.push(`missing CI job ${job}`)
    continue
  }
  if (!body.includes('components: clippy')) failures.push(`${job} does not install Clippy`)
  if (!body.includes('contents: read') || body.includes('contents: write')) {
    failures.push(`${job} must build with read-only repository contents permission`)
  }
  const supplyChainIndex = body.indexOf('node scripts/check-npm-supply-chain.cjs')
  const dependencyInstallIndex = body.indexOf('npm ci --ignore-scripts')
  if (supplyChainIndex < 0 || dependencyInstallIndex < 0 || supplyChainIndex > dependencyInstallIndex) {
    failures.push(`${job} does not run the npm malware gate before installation`)
  }
  if (!body.includes('npm rebuild esbuild')) failures.push(`${job} does not rebuild only the reviewed esbuild lifecycle script`)
  if (!body.includes('check-npm-supply-chain.cjs --installed-only')) {
    failures.push(`${job} does not scan installed npm content for ChainDrop indicators`)
  }
  if (body.includes('softprops/action-gh-release') || body.includes('gh release upload')) {
    failures.push(`${job} must not publish while build dependencies are present`)
  }
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
if (!qualityJob.includes('node scripts/check-npm-supply-chain.cjs')) {
  failures.push('quality job does not run the npm malware gate')
}
if (!qualityJob.includes('npm ci --ignore-scripts') || !qualityJob.includes('npm rebuild esbuild')) {
  failures.push('quality job does not enforce reviewed npm lifecycle scripts')
}
if (!qualityJob.includes('check-npm-supply-chain.cjs --installed-only')) {
  failures.push('quality job does not scan installed npm content for ChainDrop indicators')
}
if (!qualityJob.includes('node node_modules/playwright/cli.js install chromium') || qualityJob.includes('npx playwright')) {
  failures.push('quality job may fetch an undeclared Playwright package')
}
if (!qualityJob.includes('cargo install cargo-audit --version 0.22.2 --locked')) {
  failures.push('quality job does not install the exact reviewed cargo-audit release')
}
if (!qualityJob.includes('cargo audit --file src-tauri/Cargo.lock')) {
  failures.push('RustSec audit is not scoped to the Tauri Cargo.lock')
}
if (qualityJob.includes('checks: write') || qualityJob.includes('GITHUB_TOKEN') || /^\s+[a-z-]+:\s+write\s*$/m.test(qualityJob)) {
  failures.push('pull-request quality job exposes a write-capable token after candidate code execution')
}
const configuredAdvisories = [...qualityJob.matchAll(/--ignore\s+(RUSTSEC-\d{4}-\d{4})/g)]
  .map(match => match[1])
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

if (workflow.includes('softprops/action-gh-release')) {
  failures.push('build workflow still delegates release publication to package build jobs')
}
if (workflow.includes('::warning::')) {
  failures.push('expected code-signing fallbacks must not emit warning annotations')
}

if (workflow.includes('environment: release-r2') || workflow.includes('secrets.TAURI_SIGNING_PRIVATE_KEY') || workflow.includes('secrets.R2_')) {
  failures.push('tag-selected Build workflow must not cross the protected release-secret boundary')
}
if (workflow.includes('publish-updater:') || workflow.includes('finalize-release:')) {
  failures.push('tag-selected Build workflow still contains release publication jobs')
}

const protectedQualifyJob = workflowJobBody(protectedReleaseWorkflow, 'qualify')
const protectedStageJob = workflowJobBody(protectedReleaseWorkflow, 'stage')
const protectedPublishJob = workflowJobBody(protectedReleaseWorkflow, 'publish')
for (const token of [
  'workflow_run:',
  'workflows: [Build]',
  'ref: ${{ github.workflow_sha }}',
  'SOURCE_RUN_ID: ${{ github.event.workflow_run.id }}',
  'SOURCE_SHA: ${{ github.event.workflow_run.head_sha }}',
  'PUBLISHER_SHA: ${{ github.workflow_sha }}',
  'git rev-list --first-parent',
  'git tag --points-at "$SOURCE_SHA"',
  'publisher_sha: ${{ steps.provenance.outputs.publisher_sha }}',
  'rebuild-windows:',
  'rebuild-macos:',
  'rebuild-linux:',
  'Stage fixed-name inert release inputs at the immutable publisher revision',
  'protected-release-stage',
  'environment: release-r2',
  'Verify immutable staged inputs before any secret use',
  'Install exact integrity-pinned Tauri signer without repository code',
  'Sign exact rebuilt updater payloads and release envelope',
  'release counter exceeds the signed updater range',
  'Publish exact GitHub Release assets without replacement',
  'Publish immutable R2 objects and atomically advance updater state',
  '--if-match "$latest_etag"',
  "--if-none-match '*'",
  "cache-control 'public,max-age=31536000,immutable'",
  "cache-control 'no-cache, max-age=0, must-revalidate'",
  'TAURI_SIGNING_PRIVATE_KEY',
  'TAURI_SIGNING_PRIVATE_KEY_PASSWORD',
  'R2_ACCESS_KEY_ID',
  'R2_SECRET_ACCESS_KEY',
]) {
  if (!protectedReleaseWorkflow.includes(token)) failures.push(`protected release publication is missing ${token}`)
}
if (!protectedQualifyJob.includes('persist-credentials: false') || !protectedStageJob.includes('persist-credentials: false')) {
  failures.push('protected qualification or staging checkout persists repository credentials')
}
if (protectedPublishJob.includes('actions/checkout') || protectedPublishJob.includes('npm ci') || protectedPublishJob.includes('npm install') || protectedPublishJob.includes('npm run')) {
  failures.push('secret-bearing publisher checks out or executes repository dependency code')
}
if (protectedPublishJob.includes('--clobber')) {
  failures.push('protected publisher can replace mutable GitHub Release assets')
}
for (const job of ['qualify', 'rebuild-windows', 'rebuild-macos', 'rebuild-linux', 'stage']) {
  if (/\$\{\{\s*secrets\./.test(workflowJobBody(protectedReleaseWorkflow, job))) {
    failures.push(`${job} crosses the protected release-secret boundary`)
  }
}
if (!/\$\{\{\s*secrets\.TAURI_SIGNING_PRIVATE_KEY\s*\}\}/.test(protectedPublishJob)
  || !/\$\{\{\s*secrets\.R2_ACCESS_KEY_ID\s*\}\}/.test(protectedPublishJob)) {
  failures.push('secret-bearing publisher is missing the exact protected signing or R2 credentials')
}
if (fs.existsSync('.github/workflows/publish-release-downloads.yml')) {
  failures.push('legacy direct-dispatch release backfill workflow must remain removed')
}

const windowsJob = jobBody('build-windows')
for (const token of [
  'Build Tauri release and updater payload',
  'artifacts remain explicitly unsigned',
  '-unsigned$([IO.Path]::GetExtension($name))',
  'Generate ephemeral updater packaging key',
  'Prepare exact Windows updater payload',
  'updater-windows',
]) {
  if (!windowsJob.includes(token)) failures.push(`Windows unsigned tag build is missing ${token}`)
}
for (const token of [
  'SIGNPATH_API_TOKEN',
  'SIGNPATH_ORGANIZATION_ID',
  'SIGNPATH_PROJECT_SLUG',
  'SIGNPATH_SIGNING_POLICY_SLUG',
  'SIGNPATH_ARTIFACT_CONFIGURATION_SLUG',
  'signpath/github-action-submit-signing-request',
  'WINDOWS_CERTIFICATE',
  'WINDOWS_CERTIFICATE_PASSWORD',
  'WINDOWS_CERTIFICATE_THUMBPRINT',
  'Import-PfxCertificate',
]) {
  if (windowsJob.includes(token)) failures.push(`Windows tag-selected job still contains signing credential token ${token}`)
}
if (/\bsecrets\./.test(windowsJob)) failures.push('Windows tag-selected job must not consume repository or environment secrets')

const macJob = jobBody('build-macos')
if (!macJob.includes('runs-on: macos-15')) failures.push('macOS runner is not pinned and may migrate without review')
for (const token of [
  'Build ad-hoc signed Tauri package',
  'Build ad-hoc signed Tauri release and updater payload',
  '-adhoc',
  'Generate ephemeral updater packaging key',
  'Prepare exact macOS updater payload',
  'updater-macos',
]) {
  if (!macJob.includes(token)) failures.push(`macOS ad-hoc tag build is missing ${token}`)
}
for (const token of [
  'APPLE_CERTIFICATE',
  'APPLE_CERTIFICATE_PASSWORD',
  'APPLE_SIGNING_IDENTITY',
  'APPLE_ID',
  'APPLE_PASSWORD',
  'APPLE_TEAM_ID',
  'Import Apple signing certificate',
  'Build signed and notarized Tauri release',
]) {
  if (macJob.includes(token)) failures.push(`macOS tag-selected job still contains signing credential token ${token}`)
}
if (/\bsecrets\./.test(macJob)) failures.push('macOS tag-selected job must not consume repository or environment secrets')
if (!macJob.includes('Prepare exact macOS GitHub release asset') || !macJob.includes('release-macos')) {
  failures.push('macOS build does not stage an isolated GitHub release artifact')
}

for (const token of [
  'actions/dependency-review-action@2031cfc080254a8a887f58cffee85186f0e49e48',
  'fail-on-severity: low',
  'persist-credentials: false',
]) {
  if (!dependencyReviewWorkflow.includes(token)) failures.push(`dependency review workflow is missing ${token}`)
}

for (const link of ['PRIVACY.md', 'CODE_SIGNING_POLICY.md', 'docs/RELEASE_SIGNING.md', 'docs/DEPENDENCY_AUDIT.md']) {
  if (!readme.includes(link)) failures.push(`README does not link to ${link}`)
}
if (!signingPolicy.includes('Free code signing provided by SignPath.io, certificate by SignPath Foundation')) {
  failures.push('code signing policy is missing the SignPath Foundation attribution')
}
for (const service of ['Cloudflare', 'GitHub', 'ModelScope', 'Hugging Face']) {
  if (!privacyPolicy.includes(service)) failures.push(`privacy policy does not disclose ${service} network access`)
}
for (const obsoleteToken of ['WINDOWS_CERTIFICATE', 'Import-PfxCertificate']) {
  if (signingGuide.includes(obsoleteToken)) failures.push(`release signing guide still documents obsolete token ${obsoleteToken}`)
}

if (tauriConfig.bundle?.macOS?.signingIdentity !== '-') {
  failures.push('macOS non-release artifacts do not use an ad-hoc signing identity')
}
if (tauriConfig.bundle?.createUpdaterArtifacts !== false) {
  failures.push('ordinary builds must not require access to the protected updater signing key')
}
if (updaterBuildConfig.bundle?.createUpdaterArtifacts !== true) {
  failures.push('release builds must enable Tauri updater artifact generation')
}
const updaterTargets = updaterBuildConfig.bundle?.targets
if (!Array.isArray(updaterTargets) || !updaterTargets.includes('app')) {
  failures.push('release builds must include the macOS app target required for updater artifacts')
}
if (tauriConfig.bundle?.targets?.includes('appimage') || tauriConfig.bundle?.linux?.appimage) {
  failures.push('ordinary Linux builds must not produce the suspended AppImage package')
}
for (const staleGuideClaim of [
  'DEB 或 AppImage',
  'DEB or AppImage',
  'AppImage autostart records',
  'Linux 自动更新仅支持 AppImage',
  'Linux in-app updates are available for AppImage',
]) {
  if (guide.includes(staleGuideClaim)) failures.push(`GUIDE.md still contains suspended AppImage guidance: ${staleGuideClaim}`)
}
for (const currentGuideClaim of ['暂停发布 AppImage', 'AppImage distribution is suspended']) {
  if (!guide.includes(currentGuideClaim)) failures.push(`GUIDE.md is missing current Linux packaging guidance: ${currentGuideClaim}`)
}
if (updaterTargets?.includes('appimage') || updaterTargets?.includes('deb')) {
  failures.push('Linux packages must not enter the signed updater artifact build')
}
for (const job of ['build-linux', 'build-linux-arm64']) {
  const body = jobBody(job)
  if (!body.includes('npm run tauri build -- --bundles deb')) {
    failures.push(`${job} must build only the DEB package`)
  }
  for (const forbidden of ['AppImage', 'updater-linux-', 'src-tauri/tauri.updater.conf.json']) {
    if (body.includes(forbidden)) failures.push(`${job} still contains suspended Linux updater token ${forbidden}`)
  }
}
for (const forbidden of ['AppImage', 'updater-linux-x86_64', 'updater-linux-aarch64']) {
  if (protectedReleaseWorkflow.includes(forbidden)) {
    failures.push(`protected updater publication still contains suspended Linux updater token ${forbidden}`)
  }
}
if (!workflow.includes('npm run tauri build -- --config src-tauri/tauri.updater.conf.json')) {
  failures.push('tag builds do not use the shell-safe updater build config')
}
if (jobBody('build-macos').includes('mapfile')) {
  failures.push('macOS release preparation uses mapfile, which is unavailable in the runner Bash 3.2')
}
if (workflowJobBody(protectedReleaseWorkflow, 'rebuild-macos').includes('mapfile')) {
  failures.push('protected macOS rebuild uses mapfile, which is unavailable in the runner Bash 3.2')
}
if (!macJob.includes('Build ad-hoc signed Tauri package and updater smoke payload')) {
  failures.push('macOS pull-request builds do not exercise updater artifact generation')
}
if ((macJob.match(/npm run tauri build -- --config src-tauri\/tauri\.updater\.conf\.json/g) || []).length < 2) {
  failures.push('macOS updater build config is not used by both pull-request and ad-hoc tag paths')
}
if (!/- name: Generate ephemeral updater packaging key\r?\n\s+run: \|/.test(macJob)) {
  failures.push('macOS pull-request builds do not generate an isolated updater smoke-test key')
}
if (!/- name: Prepare exact macOS updater payload\r?\n\s+run: \|/.test(macJob)) {
  failures.push('macOS pull-request builds do not exercise updater payload preparation')
}
const updaterConfig = tauriConfig.plugins?.updater
if (!updaterConfig?.pubkey || !Array.isArray(updaterConfig.endpoints) || updaterConfig.endpoints.length !== 1) {
  failures.push('Tauri updater must use one signed production endpoint')
}
if (updaterConfig?.windows?.installMode !== 'passive') {
  failures.push('Windows updater install mode must remain passive')
}
if (tauriConfig.app?.security?.csp?.includes('api.github.com')) {
  failures.push('runtime CSP still permits the retired GitHub release-check endpoint')
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
