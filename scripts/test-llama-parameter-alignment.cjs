const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const root = path.join(__dirname, '..')
const read = (...segments) => fs.readFileSync(path.join(root, ...segments), 'utf8')
const baseline = JSON.parse(read('scripts', 'llama-parameter-baseline.json'))
const server = read('src-tauri', 'src', 'commands', 'server.rs')
const rustDefaults = read('src-tauri', 'src', 'models.rs')
const frontendDefaults = read('src', 'store', 'defaults.ts')
const validators = read('src', 'validators.ts')
const shared = read('src', 'components', 'ConfigPage', 'shared.tsx')
const english = read('src', 'i18n', 'en-US.ts')
const configPage = read('src', 'components', 'ConfigPage.tsx')
const compatibilityHook = read('src', 'components', 'ConfigPage', 'useEngineCompatibility.ts')

assert.match(baseline.upstreamCommit, /^[0-9a-f]{40}$/)
assert.match(baseline.upstreamMasterChecked, /^[0-9a-f]{40}$/)
assert.equal(baseline.schemaVersion, 2)
assert.ok(baseline.releaseSnapshot.parameterCount >= 100)
assert.equal(baseline.releaseSnapshot.parameters.length, baseline.releaseSnapshot.parameterCount)

const generationStart = server.indexOf('fn append_basic_flags')
const generationEnd = server.indexOf('fn prepare_launch')
assert.ok(generationStart >= 0 && generationEnd > generationStart, 'could not isolate Rust command generation')
const commandGeneration = server.slice(generationStart, generationEnd)
const emittedFlags = [...new Set(
  [...commandGeneration.matchAll(/["'](-{1,2}[a-zA-Z][a-zA-Z0-9-]*)["']/g)].map(match => match[1]),
)].sort()
const officialAliases = new Set(baseline.releaseSnapshot.parameters.flatMap(parameter => parameter.aliases))
const masterAliases = new Set(baseline.masterSnapshot.parameters.flatMap(parameter => parameter.aliases))
const previewFlags = new Set(baseline.masterPreviewFlags)
assert.ok(emittedFlags.length >= 80, `expected a substantial command registry, found ${emittedFlags.length}`)
for (const flag of emittedFlags) {
  assert.ok(
    officialAliases.has(flag) || previewFlags.has(flag),
    `Rust command generation emits a flag absent from ${baseline.upstreamRelease} and the reviewed master preview list: ${flag}`,
  )
  assert.ok(validators.includes(`'${flag}'`), `custom-argument conflict registry is missing generated flag ${flag}`)
}
for (const flag of previewFlags) {
  assert.ok(emittedFlags.includes(flag), `reviewed master preview flag is no longer generated: ${flag}`)
  assert.ok(masterAliases.has(flag), `reviewed preview flag is absent from the master snapshot: ${flag}`)
  assert.ok(!officialAliases.has(flag), `preview flag has reached stable and should be removed from masterPreviewFlags: ${flag}`)
}

for (const flag of [...baseline.structuredFlagsAdded, ...baseline.defaultTrueNegativeFlags]) {
  assert.ok(server.includes(`"${flag}"`), `Rust command generation is missing ${flag}`)
  assert.ok(validators.includes(`'${flag}'`), `custom-argument conflict registry is missing ${flag}`)
}

assert.match(rustDefaults, /checkpoint_min_step:\s*8192/)
assert.match(frontendDefaults, /checkpoint_min_step:\s*8192/)
assert.match(server, /config\.cache_ram\s*!=\s*8192/)
assert.match(server, /config\.checkpoint_min_step\s*!=\s*8192/)
assert.match(server, /config\.keep\s*!=\s*0/)
assert.match(server, /config\.draft_gpu_layers\s*!=\s*99/)
assert.doesNotMatch(shared, /['"]mistral['"]/, 'removed built-in template must not remain selectable')
assert.doesNotMatch(english, /apply_diff/, 'removed upstream built-in tool must not be advertised')
assert.match(server, /ENGINE_PARAMETER_UNSUPPORTED/, 'server launch must enforce detected engine capabilities')
assert.match(configPage, /useEngineCompatibility/, 'the configuration page must use engine capability negotiation')
assert.match(compatibilityHook, /probeEngineCapabilities/, 'selected configured engines must be capability-probed')
assert.match(configPage, /unsupportedEngineFlags\.length > 0/, 'unsupported active flags must block configuration save')

console.log(`llama.cpp parameter baseline ${baseline.upstreamRelease} alignment tests passed`)
