export type CanaryRolloutState = 'active' | 'promoted' | 'aborted' | 'rolled_back'

export type CanaryRequestEvidence = {
  total: number
  succeeded: number
  failed: number
  latestCompletedAt: number | null
}

export type CanaryTargetHealth = {
  instanceId: string
  status: string
  ready: boolean
}

export type CanaryAuditEvent = {
  sequence: number
  occurredAt: number
  kind: string
  summary: string
  stableEvidence: CanaryRequestEvidence | null
  candidateEvidence: CanaryRequestEvidence | null
  integrityValid: boolean
}

export type CanaryRollout = {
  id: string
  modelAlias: string
  state: CanaryRolloutState
  stableInstanceId: string
  candidateInstanceId: string
  stableRevisionId: string
  candidateRevisionId: string
  stableWeight: number
  candidateWeight: number
  createdAt: number
  updatedAt: number
  integrityValid: boolean
  drift: string[]
  canChangeTraffic: boolean
  canPromote: boolean
  canAbort: boolean
  canRollback: boolean
  stableHealth: CanaryTargetHealth
  candidateHealth: CanaryTargetHealth
  stableEvidence: CanaryRequestEvidence | null
  candidateEvidence: CanaryRequestEvidence | null
  events: CanaryAuditEvent[]
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}

function text(value: unknown, fallback = '') {
  return typeof value === 'string' ? value : fallback
}

function number(value: unknown, fallback = 0) {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function bool(value: unknown, fallback = false) {
  return typeof value === 'boolean' ? value : fallback
}

function evidence(value: unknown): CanaryRequestEvidence | null {
  if (value == null) return null
  const source = record(value)
  return {
    total: Math.max(0, number(source.total)),
    succeeded: Math.max(0, number(source.succeeded)),
    failed: Math.max(0, number(source.failed)),
    latestCompletedAt: typeof source.latestCompletedAt === 'number' ? source.latestCompletedAt : null,
  }
}

function health(value: unknown, instanceId: string): CanaryTargetHealth {
  const source = record(value)
  return {
    instanceId: text(source.instanceId, instanceId),
    status: text(source.status, 'unknown'),
    ready: bool(source.ready),
  }
}

function auditEvent(value: unknown, index: number): CanaryAuditEvent {
  const source = record(value)
  return {
    sequence: number(source.sequence, index + 1),
    occurredAt: number(source.occurredAt),
    kind: text(source.kind, 'unknown'),
    summary: text(source.summary),
    stableEvidence: evidence(source.stableEvidence),
    candidateEvidence: evidence(source.candidateEvidence),
    integrityValid: bool(source.integrityValid),
  }
}

function rolloutState(value: unknown): CanaryRolloutState {
  return value === 'active' || value === 'promoted' || value === 'aborted' || value === 'rolled_back'
    ? value
    : 'aborted'
}

export function normalizeCanaryRollout(value: unknown): CanaryRollout {
  const source = record(value)
  const stableInstanceId = text(source.stableInstanceId)
  const candidateInstanceId = text(source.candidateInstanceId)
  return {
    id: text(source.id),
    modelAlias: text(source.modelAlias),
    state: rolloutState(source.state),
    stableInstanceId,
    candidateInstanceId,
    stableRevisionId: text(source.stableRevisionId),
    candidateRevisionId: text(source.candidateRevisionId),
    stableWeight: Math.max(0, number(source.stableWeight)),
    candidateWeight: Math.max(0, number(source.candidateWeight)),
    createdAt: number(source.createdAt),
    updatedAt: number(source.updatedAt),
    integrityValid: bool(source.integrityValid),
    drift: Array.isArray(source.drift) ? source.drift.filter((item): item is string => typeof item === 'string') : [],
    canChangeTraffic: bool(source.canChangeTraffic),
    canPromote: bool(source.canPromote),
    canAbort: bool(source.canAbort),
    canRollback: bool(source.canRollback),
    stableHealth: health(source.stableHealth, stableInstanceId),
    candidateHealth: health(source.candidateHealth, candidateInstanceId),
    stableEvidence: evidence(source.stableEvidence),
    candidateEvidence: evidence(source.candidateEvidence),
    events: Array.isArray(source.events) ? source.events.map(auditEvent) : [],
  }
}

export function normalizeCanaryRollouts(value: unknown): CanaryRollout[] {
  return Array.isArray(value)
    ? value.map(normalizeCanaryRollout).filter(rollout => rollout.id.length > 0)
    : []
}

export function replaceCanaryRollout(
  rollouts: CanaryRollout[],
  value: unknown,
): CanaryRollout[] {
  const next = normalizeCanaryRollout(value)
  if (!next.id) return rollouts
  return [next, ...rollouts.filter(rollout => rollout.id !== next.id)]
    .sort((left, right) => right.updatedAt - left.updatedAt)
}

export function evidenceRate(value: CanaryRequestEvidence | null): number | null {
  return value && value.total > 0 ? value.succeeded / value.total : null
}

export function shortRevision(value: string) {
  const parts = value.split(':')
  const digest = parts[parts.length - 1] || value
  return digest.length > 12 ? digest.slice(0, 12) : digest || '—'
}
