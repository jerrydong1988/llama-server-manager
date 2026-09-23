import { useCallback, useEffect, useState } from 'react'
import type { Instance } from '../../store/types'
import { useGuideTourStore } from '../guide/guideTourStore'

export function useInstanceSelection(instances: Instance[], filteredInstances: Instance[]) {
  const [selectedId, setSelectedId] = useState('')
  const guidedId = useGuideTourStore(state => state.session?.mode === 'setup' ? state.session.instanceId : null)
  const guidedInstance = instances.find(instance => instance.id === guidedId)
  const retainedGuideId = guidedInstance?.id
  const selectedInstance = guidedInstance
    || filteredInstances.find(instance => instance.id === selectedId) || filteredInstances[0] || null
  useEffect(() => {
    if (retainedGuideId) {
      setSelectedId(retainedGuideId)
      return
    }
    if (selectedId && filteredInstances.some(instance => instance.id === selectedId)) return
    setSelectedId(filteredInstances[0]?.id || '')
  }, [filteredInstances, retainedGuideId, selectedId])
  const setSelectedInstanceId = useCallback((id: string) => {
    setSelectedId(id)
    useGuideTourStore.getState().selectInstance(id)
  }, [])
  return { selectedInstance, setSelectedInstanceId }
}
