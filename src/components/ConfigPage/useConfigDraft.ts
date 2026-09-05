import { useSyncExternalStore, type SetStateAction } from 'react'
import { useAppStore, type InstanceConfig } from '../../store'
import { migrateParameterIntent } from '../../parameterIntent'
import { normalizeModelPath } from '../../store/bootstrap'
import { isEqualValue } from './configWorkspace'

type Draft = {
  sourceConfig: InstanceConfig
  local: InstanceConfig
  baseline: InstanceConfig
  committedModelPathRef: { current: string }
  editRevisionRef: { current: number }
  saveInFlightRef: { current: boolean }
  saveStage: 'validating' | 'persisting' | null
}

// Session-only drafts survive page unmounts without persisting unsaved settings.
const drafts = new Map<string, Draft>()
const listeners = new Set<() => void>()
const subscribe = (listener: () => void) => {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}
const notify = () => listeners.forEach(listener => listener())

useAppStore.subscribe((state, previous) => {
  if (state.instances === previous.instances) return
  const instanceIds = new Set(state.instances.map(instance => instance.id))
  let removed = false
  for (const id of drafts.keys()) {
    if (!instanceIds.has(id)) {
      drafts.delete(id)
      removed = true
    }
  }
  if (removed) notify()
})

function rebaseLocal(baseline: InstanceConfig, local: InstanceConfig, persisted: InstanceConfig): InstanceConfig {
  // Preserve edits relative to the old baseline (or submitted snapshot), while
  // accepting unrelated settings changed elsewhere and backend normalization.
  const edits = Object.fromEntries((Object.keys(local) as Array<keyof InstanceConfig>)
    .filter(key => key !== 'explicit_overrides' && !isEqualValue(local[key], baseline[key]))
    .map(key => [key, local[key]]))
  const before = new Set(baseline.explicit_overrides ?? [])
  const after = new Set(local.explicit_overrides ?? [])
  const overrides = new Set(persisted.explicit_overrides ?? [])
  for (const key of before) if (!after.has(key)) overrides.delete(key)
  for (const key of after) if (!before.has(key)) overrides.add(key)
  return { ...persisted, ...edits, explicit_overrides: [...overrides] }
}

function getDraft(id: string | undefined): Draft | null {
  const instance = useAppStore.getState().instances.find(item => item.id === id)
  if (!id || !instance) return null
  let draft = drafts.get(id)
  if (draft && draft.sourceConfig !== instance.config) {
    if (!draft.saveInFlightRef.current) {
      const baseline = migrateParameterIntent(instance.config)
      const local = rebaseLocal(draft.baseline, draft.local, baseline)
      if (local.model_path !== draft.local.model_path) {
        draft.committedModelPathRef.current = normalizeModelPath(local.model_path)
      }
      draft = { ...draft, baseline, local }
    }
    draft = { ...draft, sourceConfig: instance.config }
    drafts.set(id, draft)
  }
  if (!draft) {
    const config = migrateParameterIntent(instance.config)
    draft = {
      sourceConfig: instance.config,
      local: config, baseline: config,
      committedModelPathRef: { current: normalizeModelPath(config.model_path) },
      editRevisionRef: { current: 0 }, saveInFlightRef: { current: false },
      saveStage: null,
    }
    drafts.set(id, draft)
  }
  return draft
}

const emptyRefs = {
  committedModelPathRef: { current: '' },
  editRevisionRef: { current: 0 }, saveInFlightRef: { current: false },
}

export function useConfigDraft(instanceId: string | undefined) {
  const draft = useSyncExternalStore(subscribe, () => getDraft(instanceId))
  const setSaveStage = (saveStage: Draft['saveStage']) => {
    const current = getDraft(instanceId)
    if (!instanceId || !current) return
    drafts.set(instanceId, { ...current, saveStage })
    notify()
  }
  const setBaseline = (baseline: InstanceConfig, submitted: InstanceConfig) => {
    const current = getDraft(instanceId)
    if (!instanceId || !current) return
    const local = rebaseLocal(submitted, current.local, baseline)
    if (local.model_path !== current.local.model_path) {
      current.committedModelPathRef.current = normalizeModelPath(local.model_path)
    }
    drafts.set(instanceId, { ...current, baseline, local })
    notify()
  }
  const setLocal = (action: SetStateAction<InstanceConfig | null>) => {
    const current = getDraft(instanceId)
    if (!instanceId || !current) return
    const value = typeof action === 'function' ? action(current.local) : action
    if (!value) return
    drafts.set(instanceId, { ...current, local: value })
    notify()
  }
  return {
    local: draft?.local ?? null, baseline: draft?.baseline ?? null,
    setLocal,
    setBaseline,
    committedModelPathRef: (draft ?? emptyRefs).committedModelPathRef,
    editRevisionRef: (draft ?? emptyRefs).editRevisionRef,
    saveInFlightRef: (draft ?? emptyRefs).saveInFlightRef,
    saving: draft?.saveInFlightRef.current ?? false,
    saveStage: draft?.saveStage ?? null,
    setSaveStage,
  }
}
