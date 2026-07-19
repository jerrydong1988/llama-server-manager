import type { ModelInfo } from './store/types'
import { pathDirname } from './utils/path'

export type ProjectorMatchConfidence = 'exact' | 'compatible' | 'weak' | 'unknown' | 'mismatch'

export interface ProjectorMatchResult {
  confidence: ProjectorMatchConfidence
  reason: 'base-repo' | 'source-repo' | 'identity' | 'family' | 'modality-tags' | 'same-directory' | 'unavailable' | 'source-conflict' | 'family-conflict'
}

const VISUAL_TAGS = new Set(['multimodal', 'image-text-to-text', 'any-to-any', 'vision'])

function normalizeRepo(value?: string) {
  return value
    ?.trim()
    .toLowerCase()
    .replace(/^https?:\/\//, '')
    .replace(/^www\./, '')
    .replace(/\/+$/, '')
    .replace(/\.git$/, '')
    .replace(/^huggingface\.co\//, '') || ''
}

function isModelSpecificRepo(value: string) {
  return value.split('/').filter(Boolean).length >= 2
}

function normalizeIdentity(value?: string) {
  return value?.trim().toLowerCase().replace(/[^a-z0-9]+/g, '') || ''
}

function identities(model: ModelInfo) {
  const capabilities = model.capabilities
  return [capabilities?.model_name, capabilities?.model_basename, capabilities?.base_model_name]
    .map(normalizeIdentity)
    .filter(identity => identity.length >= 6)
}

function identitiesOverlap(left: ModelInfo, right: ModelInfo) {
  const leftIdentities = identities(left)
  const rightIdentities = identities(right)
  return leftIdentities.some(a => rightIdentities.some(b => a === b || (a.length >= 10 && b.length >= 10 && (a.includes(b) || b.includes(a)))))
}

function visualTags(model: ModelInfo) {
  return new Set((model.capabilities?.tags ?? []).map(tag => tag.trim().toLowerCase()).filter(tag => VISUAL_TAGS.has(tag)))
}

function sharesVisualTag(left: ModelInfo, right: ModelInfo) {
  const leftTags = visualTags(left)
  return [...visualTags(right)].some(tag => leftTags.has(tag))
}

function isProjector(model: ModelInfo) {
  return model.file_type === 'mmproj' || Boolean(model.capabilities?.is_mmproj)
}

export function assessProjectorMatch(model: ModelInfo, projector: ModelInfo | null | undefined): ProjectorMatchResult {
  if (!projector || !isProjector(projector)) return { confidence: 'unknown', reason: 'unavailable' }

  const modelBaseRepo = normalizeRepo(model.capabilities?.base_model_repo)
  const projectorBaseRepo = normalizeRepo(projector.capabilities?.base_model_repo)
  if (modelBaseRepo && projectorBaseRepo && modelBaseRepo !== projectorBaseRepo) {
    return { confidence: 'mismatch', reason: 'source-conflict' }
  }
  if (modelBaseRepo && modelBaseRepo === projectorBaseRepo) return { confidence: 'exact', reason: 'base-repo' }

  const modelRepo = normalizeRepo(model.capabilities?.model_repo)
  const projectorRepo = normalizeRepo(projector.capabilities?.model_repo)
  if (modelRepo && modelRepo === projectorRepo && isModelSpecificRepo(modelRepo)) {
    return { confidence: 'exact', reason: 'source-repo' }
  }
  if (identitiesOverlap(model, projector)) return { confidence: 'exact', reason: 'identity' }

  const modelFamily = model.capabilities?.vision_family?.trim().toLowerCase()
  const projectorFamily = projector.capabilities?.projector_family?.trim().toLowerCase()
  if (modelFamily && projectorFamily && modelFamily !== projectorFamily) {
    return { confidence: 'mismatch', reason: 'family-conflict' }
  }
  if (modelFamily && projectorFamily && modelFamily === projectorFamily) {
    return { confidence: 'compatible', reason: 'family' }
  }
  if (sharesVisualTag(model, projector) && pathDirname(model.path) === pathDirname(projector.path)) {
    return { confidence: 'compatible', reason: 'modality-tags' }
  }
  if (pathDirname(model.path) === pathDirname(projector.path)) {
    return { confidence: 'weak', reason: 'same-directory' }
  }
  return { confidence: 'unknown', reason: 'unavailable' }
}

export function findMatchingProjector(model: ModelInfo, models: ModelInfo[]): ModelInfo | null {
  const candidates = models.filter(candidate => (
    isProjector(candidate) && pathDirname(candidate.path) === pathDirname(model.path)
  ))
  const score: Record<ProjectorMatchConfidence, number> = { mismatch: -1, unknown: 0, weak: 1, compatible: 2, exact: 3 }
  const ranked = candidates.map(projector => ({ projector, match: assessProjectorMatch(model, projector) }))
  const bestScore = Math.max(-1, ...ranked.map(candidate => score[candidate.match.confidence]))
  const best = ranked.filter(candidate => score[candidate.match.confidence] === bestScore)
  if (best.length === 1 && bestScore >= 2) return best[0].projector
  if (candidates.length === 1 && bestScore >= 1) return candidates[0]
  return null
}
