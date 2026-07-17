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

assert.match(baseline.upstreamCommit, /^[0-9a-f]{40}$/)
assert.match(baseline.upstreamMasterChecked, /^[0-9a-f]{40}$/)

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

console.log(`llama.cpp parameter baseline ${baseline.upstreamRelease} alignment tests passed`)
