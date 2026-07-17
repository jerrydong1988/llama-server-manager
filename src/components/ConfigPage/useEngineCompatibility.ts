import { useEffect, useRef, useState } from 'react'
import { useAppStore, type EngineInfo, type InstanceConfig } from '../../store'
import { findUnsupportedEngineFlags, normalizeEngineCapabilityStatus } from '../../engineCapabilities'

type CompatibilityInput = {
  local: InstanceConfig | null
  currentEngine: EngineInfo | null
  trustedEngineId: string
}

export function useEngineCompatibility({ local, currentEngine, trustedEngineId }: CompatibilityInput) {
  const generateCommand = useAppStore(state => state.generateCommand)
  const probeEngineCapabilities = useAppStore(state => state.probeEngineCapabilities)
  const [unsupportedEngineFlags, setUnsupportedEngineFlags] = useState<string[]>([])
  const [probingEngineCompatibility, setProbingEngineCompatibility] = useState(false)
  const [autoProbeFailedFor, setAutoProbeFailedFor] = useState<string | null>(null)
  const mountedRef = useRef(true)
  const probeTargetRef = useRef<string | null>(null)
  const probeInFlightRef = useRef<Set<string>>(new Set())

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  const status = normalizeEngineCapabilityStatus(currentEngine?.capabilities)
  const isTrustedSelection = Boolean(currentEngine && currentEngine.id === trustedEngineId)
  const capabilityProbeRequired = isTrustedSelection
    && status === 'unprobed'
    && autoProbeFailedFor !== currentEngine?.id

  useEffect(() => {
    if (!currentEngine || !capabilityProbeRequired) {
      probeTargetRef.current = null
      setProbingEngineCompatibility(false)
      if (status !== 'unprobed') setAutoProbeFailedFor(null)
      return
    }
    if (probeInFlightRef.current.has(currentEngine.id)) {
      probeTargetRef.current = currentEngine.id
      setProbingEngineCompatibility(true)
      return
    }

    const engineId = currentEngine.id
    probeInFlightRef.current.add(engineId)
    probeTargetRef.current = engineId
    setProbingEngineCompatibility(true)
    void probeEngineCapabilities(engineId)
      .catch(() => {
        if (mountedRef.current && probeTargetRef.current === engineId) {
          setAutoProbeFailedFor(engineId)
        }
      })
      .finally(() => {
        probeInFlightRef.current.delete(engineId)
        if (mountedRef.current && probeTargetRef.current === engineId) {
          setProbingEngineCompatibility(false)
        }
      })
  }, [capabilityProbeRequired, currentEngine, probeEngineCapabilities, status])

  useEffect(() => {
    if (!local || !currentEngine || status !== 'detected') {
      setUnsupportedEngineFlags([])
      return
    }
    let cancelled = false
    const timer = setTimeout(() => {
      void generateCommand(local, currentEngine.exe)
        .then(command => {
          if (!cancelled) {
            setUnsupportedEngineFlags(findUnsupportedEngineFlags(command, currentEngine.capabilities))
          }
        })
        .catch(() => {
          if (!cancelled) setUnsupportedEngineFlags([])
        })
    }, 180)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [currentEngine, generateCommand, local, status])

  return {
    unsupportedEngineFlags,
    setUnsupportedEngineFlags,
    probingEngineCompatibility,
    capabilityProbeRequired,
  }
}
