import { useSyncExternalStore, type SetStateAction } from 'react'
import { useAppStore, type InstanceConfig } from '../../store'
import { migrateParameterIntent } from '../../parameterIntent'
import { normalizeModelPath } from '../../store/bootstrap'

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

useAppStore.subscribe(state => {
  let removed = false
  for (const id of drafts.keys()) {
    if (!state.instances.some(instance => instance.id === id)) {
      drafts.delete(id)
      removed = true
    }
  }
  if (removed) notify()
})

function getDraft(id: string | undefined): Draft | null {
  const instance = useAppStore.getState().instances.find(item => item.id === id)
  if (!id || !instance) return null
  let draft = drafts.get(id)
  if (draft && draft.sourceConfig !== instance.config) {
    if (!draft.saveInFlightRef.current && JSON.stringify(draft.local) === JSON.stringify(draft.baseline)) {
      draft = undefined
    } else {
      draft = { ...draft, sourceConfig: instance.config }
      drafts.set(id, draft)
    }
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
  const setField = (field: 'local' | 'baseline', action: SetStateAction<InstanceConfig | null>) => {
    const current = getDraft(instanceId)
    if (!instanceId || !current) return
    const value = typeof action === 'function' ? action(current[field]) : action
    if (!value) return
    drafts.set(instanceId, { ...current, [field]: value })
    notify()
  }
  return {
    local: draft?.local ?? null, baseline: draft?.baseline ?? null,
    setLocal: (action: SetStateAction<InstanceConfig | null>) => setField('local', action),
    setBaseline: (action: SetStateAction<InstanceConfig | null>) => setField('baseline', action),
    committedModelPathRef: (draft ?? emptyRefs).committedModelPathRef,
    editRevisionRef: (draft ?? emptyRefs).editRevisionRef,
    saveInFlightRef: (draft ?? emptyRefs).saveInFlightRef,
    saving: draft?.saveInFlightRef.current ?? false,
    saveStage: draft?.saveStage ?? null,
    setSaveStage,
  }
}
