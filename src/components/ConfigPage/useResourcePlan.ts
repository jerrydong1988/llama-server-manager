import { useCallback, useEffect, useRef, useState, type MutableRefObject } from 'react'
import type { InstanceConfig, ResourcePlan } from '../../store'

export function useResourcePlan({
  config,
  engineBackend,
  editRevisionRef,
  planResources,
}: {
  config: InstanceConfig | null
  engineBackend: string
  editRevisionRef: MutableRefObject<number>
  planResources: (config: InstanceConfig, engineBackend: string) => Promise<ResourcePlan>
}) {
  const [resourcePlan, setResourcePlan] = useState<ResourcePlan | null>(null)
  const [resourcePlanRevision, setResourcePlanRevision] = useState<number | null>(null)
  const [resourcePlanLoading, setResourcePlanLoading] = useState(false)
  const [resourcePlanError, setResourcePlanError] = useState(false)
  const requestRef = useRef(0)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      requestRef.current += 1
    }
  }, [])

  const runResourcePlan = useCallback(async (
    targetConfig: InstanceConfig,
    targetEngineBackend: string,
    expectedRevision: number,
    clearCurrent = true,
  ) => {
    const requestId = ++requestRef.current
    if (clearCurrent) {
      setResourcePlan(null)
      setResourcePlanRevision(null)
    }
    setResourcePlanLoading(true)
    setResourcePlanError(false)
    try {
      const plan = await planResources(targetConfig, targetEngineBackend)
      const stale = requestId !== requestRef.current || editRevisionRef.current !== expectedRevision
      if (!stale && mountedRef.current) {
        setResourcePlan(plan)
        setResourcePlanRevision(expectedRevision)
      }
      return { plan, stale }
    } catch (error) {
      if (requestId === requestRef.current && mountedRef.current) {
        setResourcePlanError(true)
        setResourcePlan(null)
        setResourcePlanRevision(null)
      }
      throw error
    } finally {
      if (requestId === requestRef.current && mountedRef.current) setResourcePlanLoading(false)
    }
  }, [editRevisionRef, planResources])

  useEffect(() => {
    requestRef.current += 1
    setResourcePlan(null)
    setResourcePlanRevision(null)
    setResourcePlanError(false)
    if (!config) {
      setResourcePlanLoading(false)
      return
    }
    setResourcePlanLoading(true)
    const expectedRevision = editRevisionRef.current
    const timer = setTimeout(() => {
      void runResourcePlan(config, engineBackend, expectedRevision).catch(() => {})
    }, 350)
    return () => clearTimeout(timer)
  }, [config, editRevisionRef, engineBackend, runResourcePlan])

  const refreshResourcePlan = useCallback(() => {
    if (!config) return
    void runResourcePlan(config, engineBackend, editRevisionRef.current).catch(() => {})
  }, [config, editRevisionRef, engineBackend, runResourcePlan])
  const currentResourcePlan = resourcePlanRevision === editRevisionRef.current ? resourcePlan : null

  return { currentResourcePlan, resourcePlanLoading, resourcePlanError, runResourcePlan, refreshResourcePlan }
}
