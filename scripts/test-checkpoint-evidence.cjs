const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs'), os = require('node:os'), path = require('node:path')
const { inspectSlotFile, assertFullRead, assertInference } = require('./lib/checkpoint-evidence.cjs')
const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b }
const u64 = n => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b }
const blob = b => Buffer.concat([u64(b.length), b])
const context = Buffer.from('target KV/recurrent')
const draft = Buffer.from('draft KV/recurrent')
const spec = Buffer.from('nonempty speculative state')
function fixture(target = context, speculative = spec) {
  return Buffer.concat([u32(0x67677371), u32(4), u32(2), u32(1), u32(2), target,
    Buffer.from('SCKP'), u32(1), u32(1), u64(2), u32(0), u32(1), blob(context), blob(draft), blob(spec),
    Buffer.from('SDFT'), u32(1), blob(draft), blob(speculative)])
}
function inspect(bytes) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'lsm-state-evidence-')), file = path.join(root, 'slot.bin')
  try { fs.writeFileSync(file, bytes); return inspectSlotFile(file) }
  finally { fs.rmSync(root, { recursive: true, force: true }) }
}
test('records target, checkpoint, full draft and nonempty speculative blobs separately', () => {
  const parsed = inspect(fixture())
  assert.equal(parsed.targetStateBytes, context.length)
  assert.equal(parsed.checkpoints.length, 1)
  assert.equal(parsed.checkpoints[0].draft.bytes, draft.length)
  assert.equal(parsed.liveDraft.speculative.bytes, spec.length)
})
test('finds a split magic across scan chunks and records an empty speculative payload', () => {
  const target = Buffer.alloc(1024 * 1024 - 2)
  target.write('SDFT invalid tensor magic')
  const parsed = inspect(fixture(target, Buffer.alloc(0)))
  assert.equal(parsed.targetStateBytes, target.length)
  assert.equal(parsed.liveDraft.speculative.bytes, 0)
})
test('rejects truncation, invalid lengths, versions and trailing bytes', () => {
  for (const cut of [1, 8, spec.length + 8]) assert.throws(() => inspect(fixture().subarray(0, -cut)))
  const huge = fixture(); huge.writeBigUInt64LE(2n ** 63n, huge.indexOf('SDFT') + 8)
  assert.throws(() => inspect(huge))
  const version = fixture(); version.writeUInt32LE(99, version.indexOf('SDFT') + 4)
  assert.throws(() => inspect(version))
  assert.throws(() => inspect(Buffer.concat([fixture(), Buffer.from('extra')])))
})
test('does not confuse HTTP 200 with complete native state restoration', () => {
  const saved = { id_slot: 0, filename: 'zero.bin', n_written: 500, n_saved: 20 }
  const restored = { id_slot: 0, filename: 'zero.bin', n_read: 500, n_restored: 20 }
  assertFullRead(saved, restored)
  assert.throws(() => assertFullRead(saved, { ...restored, n_read: 300 }), /complete saved state/)
  assert.throws(() => assertFullRead(saved, { ...restored, id_slot: 1 }), /wrong slot/)
  assert.throws(() => assertFullRead(saved, { ...restored, filename: 'one.bin' }), /wrong file/)
})
test('requires true generation, matching output, draft counters and cache reuse', () => {
  const good = { tokens: [3, 9, 1], content: '391', timings: { cache_n: 100, draft_n: 9, draft_n_accepted: 6 } }
  assertInference(good, good, { draft: true, reuse: true })
  assert.throws(() => assertInference(good, good, { marker: '437' }), /expected sentinel/)
  assert.throws(() => assertInference(good, good, { reuse: true, minimumReuse: 101 }), /insufficient prompt reuse/)
  for (const bad of [{ ...good, tokens: [] }, { ...good, content: '/' }, { ...good, timings: { ...good.timings, cache_n: 0 } },
    { ...good, timings: { ...good.timings, draft_n_accepted: 0 } }]) {
    assert.throws(() => assertInference(good, bad, { draft: true, reuse: true }))
  }
})
