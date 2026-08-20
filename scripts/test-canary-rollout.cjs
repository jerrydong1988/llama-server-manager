const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import {
    evidenceRate,
    normalizeCanaryRollout,
    normalizeCanaryRollouts,
    replaceCanaryRollout,
    shortRevision,
  } from './src/components/CanaryRollout/canaryRollout'

  const raw = {
    id: 'rollout-1',
    modelAlias: 'public-model',
    state: 'active',
    stableInstanceId: 'stable',
    candidateInstanceId: 'candidate',
    stableRevisionId: 'urn:stable:1234567890abcdef',
    candidateRevisionId: 'urn:candidate:fedcba0987654321',
    stableWeight: 90,
    candidateWeight: 10,
    createdAt: 100,
    updatedAt: 200,
    integrityValid: true,
    drift: [],
    canChangeTraffic: true,
    canPromote: true,
    canAbort: true,
    canRollback: false,
    stableHealth: { instanceId: 'stable', status: 'ready', ready: true },
    candidateHealth: { instanceId: 'candidate', status: 'ready', ready: true },
    stableEvidence: { total: 10, succeeded: 9, failed: 1, latestCompletedAt: 150, ttftP95Ms: 1200, queueWaitP95Ms: 80, cacheReuseBasisPoints: 3750 },
    candidateEvidence: { total: 2, succeeded: 2, failed: 0, latestCompletedAt: 160, ttftP95Ms: 900, queueWaitP95Ms: 20, cacheReuseBasisPoints: 5000 },
    events: [{ sequence: 1, occurredAt: 100, kind: 'created', summary: 'created', integrityValid: true }],
  }

  const rollout = normalizeCanaryRollout(raw)
  assert.equal(rollout.state, 'active')
  assert.equal(rollout.candidateHealth.ready, true)
  assert.equal(rollout.events[0].integrityValid, true)
  assert.equal(evidenceRate(rollout.stableEvidence), 0.9)
  assert.equal(rollout.stableEvidence?.ttftP95Ms, 1200)
  assert.equal(rollout.stableEvidence?.queueWaitP95Ms, 80)
  assert.equal(rollout.stableEvidence?.cacheReuseBasisPoints, 3750)
  assert.equal(evidenceRate({ total: 0, succeeded: 0, failed: 0, latestCompletedAt: null, ttftP95Ms: null, queueWaitP95Ms: null, cacheReuseBasisPoints: null }), null)
  assert.equal(shortRevision(raw.stableRevisionId), '1234567890ab')

  const malformed = normalizeCanaryRollout({ id: 'safe', state: 'unexpected', drift: [1, 'kept'] })
  assert.equal(malformed.state, 'aborted')
  assert.deepEqual(malformed.drift, ['kept'])
  assert.equal(malformed.canPromote, false)

  assert.deepEqual(normalizeCanaryRollouts(null), [])
  const replaced = replaceCanaryRollout([rollout], { ...raw, state: 'promoted', updatedAt: 300 })
  assert.equal(replaced.length, 1)
  assert.equal(replaced[0].state, 'promoted')
`

const bundled = esbuild.buildSync({
  bundle: true,
  format: 'cjs',
  platform: 'node',
  packages: 'external',
  write: false,
  stdin: {
    contents: entry,
    resolveDir: process.cwd(),
    sourcefile: 'canary-rollout.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'canary-rollout.test.cjs'))
testModule.filename = path.join(process.cwd(), 'canary-rollout.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

const panel = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'CanaryRollout', 'CanaryRolloutPanel.tsx'),
  'utf8',
)
assert.match(panel, /create_canary_rollout/, 'panel must expose explicit canary creation')
assert.match(panel, /observe_canary_rollout/, 'panel must expose explicit observation')
assert.match(panel, /set_canary_weight/, 'panel must expose explicit traffic changes')
assert.match(panel, /promote_canary_rollout/, 'panel must expose explicit promotion')
assert.match(panel, /abort_canary_rollout/, 'panel must expose explicit abort')
assert.match(panel, /rollback_canary_rollout/, 'panel must expose explicit rollback')
assert.match(panel, /window\.confirm\(labels\.confirmPromote\)/, 'promotion must require confirmation')
assert.match(panel, /!rollout\.candidateHealth\.ready/, 'promotion must be gated on candidate health')
assert.match(panel, /rollout\.drift\.length > 0/, 'drift must be visible')
assert.match(panel, /rollout\.events/, 'audit evidence must be visible')
assert.match(panel, /value\.ttftP95Ms/, 'canary evidence must expose target TTFT')
assert.match(panel, /value\.queueWaitP95Ms/, 'canary evidence must expose target queue wait')
assert.match(panel, /value\.cacheReuseBasisPoints/, 'canary evidence must expose observed cache reuse')

console.log('Canary rollout view-model and action gates passed.')
