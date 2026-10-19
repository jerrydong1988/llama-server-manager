// External management changes invalidate queued GUI snapshots, while saves made
// by this window retain their existing coalescing behavior.
let revision = 0
let savedSignature: string | null = null
let hasDirtyDrafts = () => false
export const registerManagementDraftCheck = (check: () => boolean) => { hasDirtyDrafts = check }
type ConfigState = {
  instances: { id: string; config: unknown }[]
  modelDirs: string[]
  engineDirs: string[]
  defaultEngineId: string | null
}
function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical)
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, item]) => [key, canonical(item)]))
  }
  return value
}
export const managementSignature = (state: ConfigState) => JSON.stringify(canonical({
  instances: state.instances.map(instance => ({ id: instance.id, config: instance.config })),
  modelDirs: state.modelDirs,
  engineDirs: state.engineDirs,
  defaultEngineId: state.defaultEngineId || '',
}))
export const markManagementSaved = (signature: string) => { savedSignature = signature }
export const hasUnsavedManagementChanges = (state: ConfigState) => (
  hasDirtyDrafts() || (savedSignature !== null && savedSignature !== managementSignature(state))
)
export const getManagementRevision = () => revision
export const setManagementRevision = (value: number | undefined) => {
  revision = typeof value === 'number' && Number.isSafeInteger(value) ? value : 0
}
