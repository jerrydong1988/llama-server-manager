import type { ModelInfo } from './store/types'
import { pathDirname } from './utils/path'

export function findMatchingProjector(model: ModelInfo, models: ModelInfo[]): ModelInfo | null {
  const candidates = models.filter(candidate => (
    candidate.file_type === 'mmproj' && pathDirname(candidate.path) === pathDirname(model.path)
  ))
  const visionFamily = model.capabilities?.vision_family?.trim().toLowerCase()
  if (!visionFamily) return candidates[0] ?? null
  const exact = candidates.find(candidate => (
    candidate.capabilities?.projector_family?.trim().toLowerCase() === visionFamily
  ))
  if (exact) return exact
  if (candidates.length === 1 && !candidates[0].capabilities?.projector_family?.trim()) {
    return candidates[0]
  }
  return null
}
