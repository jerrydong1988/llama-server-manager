import { useCallback, useLayoutEffect, useMemo, useRef } from 'react'
import { useAppStore, type AppState, type Instance } from '../../store'
import { pathsEqual } from '../../utils/path'
import { useGuideTourStore } from '../guide/guideTourStore'

export function updateInstanceListView(changes: Partial<AppState['instanceListView']>) {
  useAppStore.setState(state => ({ instanceListView: { ...state.instanceListView, ...changes } }))
}

export function useInstanceSelection(instances: Instance[], defaultEngineId: string | null, engineNameFor: (instance: Instance) => string) {
  const { selectedId, search, statusFilter, engineFilter } = useAppStore(state => state.instanceListView)
  const matches = useMemo(() => {
    const query = search.trim().toLowerCase()
    return {
      status: (inst: Instance) => statusFilter === 'all' || (statusFilter === 'running') === (inst.status === 'running'),
      engine: (inst: Instance) => engineFilter === 'all' || pathsEqual(inst.config.engine_id || defaultEngineId || '', engineFilter),
      search: (inst: Instance) => !query || inst.name.toLowerCase().includes(query)
        || inst.model.toLowerCase().includes(query) || engineNameFor(inst).toLowerCase().includes(query)
        || String(inst.config.port).includes(query),
    }
  }, [search, statusFilter, engineFilter, defaultEngineId, engineNameFor])
  const filteredInstances = useMemo(() => instances.filter(inst => matches.status(inst) && matches.engine(inst) && matches.search(inst)), [instances, matches])
  const hydrated = useAppStore(state => state.instancesHydrated)
  const restoreId = useRef(selectedId)
  const selectedRowRef = useRef<HTMLDivElement>(null)
  const guidedId = useGuideTourStore(state => state.session?.mode === 'setup' ? state.session.instanceId : null)
  const guidedInstance = instances.find(instance => instance.id === guidedId)
  const selectedInstance = guidedInstance
    || filteredInstances.find(instance => instance.id === selectedId) || filteredInstances[0] || null
  const resolvedId = selectedInstance?.id ?? null

  useLayoutEffect(() => {
    if (!hydrated) return
    if (restoreId.current) {
      const target = guidedInstance ?? instances.find(instance => instance.id === restoreId.current)
      // A saved port or a runtime status change can hide the returning instance.
      if (target && !filteredInstances.includes(target)) {
        updateInstanceListView({
          search: matches.search(target) ? search : '',
          statusFilter: matches.status(target) ? statusFilter : 'all',
          engineFilter: matches.engine(target) ? engineFilter : 'all',
        })
        return
      }
      restoreId.current = null
      selectedRowRef.current?.scrollIntoView({ block: 'nearest' })
    }
    if (selectedId !== resolvedId) updateInstanceListView({ selectedId: resolvedId })
  }, [filteredInstances, guidedInstance, hydrated, instances, resolvedId, selectedId, matches, search, statusFilter, engineFilter])

  const setSelectedInstanceId = useCallback((id: string) => {
    updateInstanceListView({ selectedId: id })
    useGuideTourStore.getState().selectInstance(id)
  }, [])
  return { search, statusFilter, engineFilter, filteredInstances, selectedInstance, setSelectedInstanceId, selectedRowRef }
}
