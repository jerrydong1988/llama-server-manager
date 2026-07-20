const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const read = relative => fs.readFileSync(path.join(root, relative), 'utf8')
const catalog = read('src/parameterCatalog.ts')
const sections = read('src/components/ConfigPage/sections.tsx')
const server = read('src-tauri/src/commands/server.rs')
const validators = read('src/validators.ts')
const zh = read('src/i18n/zh-CN.ts')
const en = read('src/i18n/en-US.ts')
const baseline = JSON.parse(read('scripts/llama-parameter-baseline.json'))

const upstreamFlags = new Set()
const collectFlags = value => {
  if (!value || typeof value !== 'object') return
  if (Array.isArray(value)) {
    value.forEach(collectFlags)
    return
  }
  if (Array.isArray(value.aliases)) value.aliases.forEach(flag => upstreamFlags.add(flag))
  Object.values(value).forEach(collectFlags)
}
collectFlags(baseline)

const catalogFlags = new Set(
  [...catalog.matchAll(/flags:\s*\[([^\]]*)\]/g)]
    .flatMap(match => [...match[1].matchAll(/['"](-{1,2}[a-z0-9-]+)['"]/gi)].map(flag => flag[1])),
)
for (const flag of catalogFlags) {
  assert.equal(upstreamFlags.has(flag), true, `parameter catalog flag is absent from the pinned llama.cpp baseline: ${flag}`)
}

for (const key of [
  'gpu_layers', 'ctx_size', 'threads', 'threads_batch', 'threads_http', 'parallel', 'draft_gpu_layers',
  'image_min_tokens', 'image_max_tokens', 'no_mmap', 'no_repack',
  'no_kv_offload', 'no_mmproj_offload', 'no_ui', 'perf',
]) {
  assert.match(catalog, new RegExp(`\\b${key}:\\s*\\{`), `missing parameter-catalog entry for ${key}`)
}

for (const [positive, negative] of [
  ['--mmap', '--no-mmap'],
  ['--repack', '--no-repack'],
  ['--kv-offload', '--no-kv-offload'],
  ['--mmproj-offload', '--no-mmproj-offload'],
  ['--ui', '--no-ui'],
  ['--perf', '--no-perf'],
  ['--spec-draft-backend-sampling', '--no-spec-draft-backend-sampling'],
]) {
  assert.equal(server.includes(`"${positive}"`), true, `backend cannot emit positive form ${positive}`)
  assert.equal(server.includes(`"${negative}"`), true, `backend cannot emit negative form ${negative}`)
  assert.equal(validators.includes(`'${positive}'`), true, `custom-argument validator is missing ${positive}`)
  assert.equal(validators.includes(`'${negative}'`), true, `custom-argument validator is missing ${negative}`)
}

assert.doesNotMatch(sections, /--sampling-backend/, 'obsolete sampling-backend spelling must not return')
assert.doesNotMatch(sections, /--cache-type-[kv]-draft/, 'obsolete draft cache flag spelling must not return')
assert.doesNotMatch(sections, /--n-gpu-layers-draft/, 'obsolete draft GPU flag spelling must not return')
assert.match(sections, /<IntentNum[\s\S]*gpu_layers/, 'automatic GPU layers must expose inherited/manual intent')
assert.match(sections, /<IntentNum[\s\S]*ctx_size/, 'automatic context size must expose inherited/manual intent')
assert.match(sections, /value=\{hasExplicitOverride\(local, 'perf'\) \? local\.perf : true\}/, 'inherited performance timing must display the source-verified effective enabled state')
assert.match(catalog, /!!config\.mmproj_path\.trim\(\)[\s\S]*!!config\.mmproj_url\.trim\(\)/, 'an explicitly selected projector must keep projector-offload controls active even when auto discovery is off')

for (const staleClaim of [
  /20-30[^'\n]*token/i,
  /5-10%/,
  /physical-core-count\s*[×x]\s*0\.8/i,
  /物理核数\*0\.8/,
  /99=all/i,
  /99\s*=\s*全部/,
  /2-4× faster/i,
  /2-4 \u500d\u66f4\u5feb/i,
]) {
  assert.doesNotMatch(`${zh}\n${en}`, staleClaim, `unverified absolute tooltip claim remains: ${staleClaim}`)
}

assert.equal(zh.includes("noMmap: '\\u5185\\u5B58\\u6620\\u5C04 (mmap)'"), true, 'Chinese inverse mmap label must be expressed as a positive capability')
assert.match(en, /Memory Mapping \(mmap\)/, 'English inverse mmap label must be expressed as a positive capability')

console.log('Parameter catalog, inverse-toggle, and tooltip accuracy regression checks passed.')
