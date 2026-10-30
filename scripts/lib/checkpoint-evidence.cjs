// Bounded readers for opt-in native acceptance. This is evidence tooling, not
// a replacement for the manager's manifest/hash/round-trip restore gates.
const fs = require('node:fs')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')

function sha256(file) {
  const fd = fs.openSync(file, 'r'), hash = crypto.createHash('sha256'), chunk = Buffer.alloc(1024 * 1024)
  try {
    for (;;) {
      const n = fs.readSync(fd, chunk, 0, chunk.length, null)
      if (!n) return hash.digest('hex')
      hash.update(chunk.subarray(0, n))
    }
  } finally { fs.closeSync(fd) }
}

function inspectSlotFile(file) {
  const fd = fs.openSync(file, 'r'), size = fs.fstatSync(fd).size
  const read = (offset, length) => {
    assert.ok(Number.isSafeInteger(offset) && offset >= 0 && offset + length <= size, 'truncated slot file')
    const b = Buffer.alloc(length)
    assert.equal(fs.readSync(fd, b, 0, length, offset), length)
    return b
  }
  const u32 = offset => read(offset, 4).readUInt32LE()
  const u64 = offset => {
    const n = read(offset, 8).readBigUInt64LE()
    assert.ok(n <= BigInt(Number.MAX_SAFE_INTEGER), 'unsafe blob length')
    return Number(n)
  }
  const blob = offset => {
    const bytes = u64(offset)
    assert.ok(bytes <= size - offset - 8, 'blob exceeds file')
    return { offset: offset + 8, bytes, end: offset + 8 + bytes }
  }
  const draft = offset => {
    assert.equal(read(offset, 4).toString(), 'SDFT')
    assert.equal(u32(offset + 4), 1, 'unsupported SDFT version')
    const state = blob(offset + 8), speculative = blob(state.end)
    assert.equal(speculative.end, size, 'unparsed trailing bytes')
    return { offset, version: 1, state, speculative }
  }
  try {
    assert.ok(size >= 12, 'truncated slot header')
    const header = read(0, 12), packedTokens = u32(8), headerEnd = 12 + 4 * packedTokens
    assert.equal(header.readUInt32LE(0), 0x67677371, 'not a GGSQ slot file')
    assert.equal(header.readUInt32LE(4), 4, 'unsupported sequence-state version')
    assert.ok(headerEnd < size, 'no target context state')
    const candidates = [], scan = Buffer.alloc(1024 * 1024 + 3)
    let overlap = 0
    for (let at = headerEnd; at < size;) {
      const n = fs.readSync(fd, scan, overlap, 1024 * 1024, at)
      if (!n) break
      const bytes = scan.subarray(0, overlap + n)
      for (const magic of ['SCKP', 'SDFT']) {
        for (let index = bytes.indexOf(magic); index !== -1; index = bytes.indexOf(magic, index + 1)) {
          const offset = at - overlap + index
          try {
            let cursor = offset, checkpoints = [], liveDraft = null
            if (magic === 'SCKP') {
              assert.equal(u32(cursor + 4), 1)
              const count = u32(cursor + 8); assert.ok(count > 0 && count <= 1024)
              cursor += 12
              for (let i = 0; i < count; i++) {
                const tokens = u64(cursor), posMin = read(cursor + 8, 4).readInt32LE(), posMax = read(cursor + 12, 4).readInt32LE()
                assert.ok(tokens > 0 && tokens <= packedTokens && posMin >= -1 && posMax >= posMin)
                const target = blob(cursor + 16), d = blob(target.end), speculative = blob(d.end)
                assert.ok(target.bytes > 0)
                checkpoints.push({ tokens, posMin, posMax, target, draft: d, speculative }); cursor = speculative.end
              }
            }
            if (cursor < size) liveDraft = draft(cursor)
            else assert.equal(cursor, size)
            candidates.push({ offset, checkpoints, liveDraft })
          } catch { /* magic in tensor data is not a structural appendix */ }
          assert.ok(candidates.length <= 16, 'ambiguous appendix candidates')
        }
      }
      overlap = Math.min(3, bytes.length); bytes.copy(scan, 0, bytes.length - overlap); at += n
    }
    const chained = candidates.filter(c => c.checkpoints.length)
    const usable = chained.length ? chained : candidates
    assert.equal(usable.length, 1, 'missing, unsupported or ambiguous slot appendices')
    const appendix = usable[0]
    return { bytes: size, packedTokens, sequenceVersion: 4, targetStateBytes: appendix.offset - headerEnd,
      mainBytes: appendix.offset, checkpointVersion: appendix.checkpoints.length ? 1 : null,
      checkpoints: appendix.checkpoints, liveDraft: appendix.liveDraft }
  } finally { fs.closeSync(fd) }
}

function assertFullRead(saved, restored) {
  assert.ok(Number.isInteger(saved.id_slot) && saved.filename, 'missing saved slot identity')
  assert.equal(restored.id_slot, saved.id_slot, 'restored the wrong slot')
  assert.equal(restored.filename, saved.filename, 'restored the wrong file')
  assert.equal(restored.n_read, saved.n_written, 'HTTP success did not consume the complete saved state')
  assert.equal(restored.n_restored, saved.n_saved, 'restored token count mismatch')
}

function assertInference(reference, actual, { draft = false, draftCounts = 'exact', reuse = false, minimumReuse = 1, marker = '391' } = {}) {
  assert.ok(Array.isArray(actual.tokens) && actual.tokens.length > 0, 'missing generated token evidence')
  assert.deepEqual(actual.tokens, reference.tokens, 'deterministic inference changed')
  assert.equal(actual.content, reference.content)
  assert.ok(actual.content.includes(marker), 'expected sentinel was not generated')
  if (reuse) assert.ok(actual.timings.cache_n >= minimumReuse, 'insufficient prompt reuse')
  if (draft) {
    assert.ok(['exact', 'rate'].includes(draftCounts), 'unknown draft comparison policy')
    assert.ok(actual.timings.draft_n > 0 && actual.timings.draft_n_accepted > 0, 'no accepted speculative tokens')
    for (const t of [actual.timings, reference.timings])
      assert.ok(Number.isSafeInteger(t.draft_n) && Number.isSafeInteger(t.draft_n_accepted)
        && t.draft_n > 0 && t.draft_n_accepted >= 0 && t.draft_n_accepted <= t.draft_n, 'invalid draft counters')
    if (draftCounts === 'exact') {
      assert.equal(actual.timings.draft_n, reference.timings.draft_n)
      assert.equal(actual.timings.draft_n_accepted, reference.timings.draft_n_accepted)
    } else {
      // For same-process health checks, ngram learning can change the amount
      // drafted. Fresh-process restore assertions retain the exact default.
      assert.equal(actual.timings.draft_n_accepted / actual.timings.draft_n,
        reference.timings.draft_n_accepted / reference.timings.draft_n, 'draft acceptance rate changed')
    }
  }
}

// Manager telemetry is host-wide. Label the sampled maxima accordingly; these
// are neither per-process GPU peaks nor upper bounds on native allocations.
function memorySampler(client, id) {
  const summary = { scope: 'host-wide sampled maxima, not allocation bounds', samples: 0,
    host_ram_mb: 0, host_vram_mb: 0, errors: 0, interval_ms: 2000 }
  let pending = null
  const timer = setInterval(() => {
    if (pending) return
    pending = client.call('instances.health', { id }).then(h => {
      const s = h.running && h.monitoring?.system
      if (s) {
        summary.samples++
        summary.host_ram_mb = Math.max(summary.host_ram_mb, s.system_memory_used_mb || 0)
        summary.host_vram_mb = Math.max(summary.host_vram_mb, s.vram_used_mb || 0)
      }
    }).catch(() => { summary.errors++ }).finally(() => { pending = null })
  }, summary.interval_ms)
  return { summary, stop: async () => { clearInterval(timer); if (pending) await pending } }
}

function processMemory(pid) {
  assert.ok(Number.isInteger(pid) && pid > 0)
  const output = require('node:child_process').execFileSync('powershell.exe', ['-NoProfile', '-Command',
    `Get-Process -Id ${pid} -ErrorAction Stop | Select-Object Id,WorkingSet64,PeakWorkingSet64,PrivateMemorySize64 | ConvertTo-Json -Compress`],
  { windowsHide: true, encoding: 'utf8', timeout: 15000 })
  return JSON.parse(output)
}

// Native counters are cumulative within one engine process. Keep each mode
// separate: an aggregate acceptance rate cannot prove a mixed pipeline ran.
function speculativeTrace(log) {
  const statistics = {}, checkpointRollbacks = []
  for (const match of log.matchAll(/statistics\s+([\w-]+):[^\r\n]*#gen tokens\s*=\s*(\d+),\s*#acc tokens\s*=\s*(\d+)/g)) {
    statistics[match[1]] = { generated: Number(match[2]), accepted: Number(match[3]) }
  }
  for (const match of log.matchAll(/accepted\s+(\d+)\/\s*(\d+) draft tokens \(restore checkpoint\)/g)) {
    checkpointRollbacks.push({ accepted: Number(match[1]), drafted: Number(match[2]) })
  }
  return { statistics, checkpointRollbacks }
}

module.exports = { sha256, inspectSlotFile, assertFullRead, assertInference, memorySampler, processMemory, speculativeTrace }
