import { useEffect, useRef, useState } from 'react'
import { useAppStore, type EngineInfo, type InstanceConfig } from '../../store'
import type { GeneratedServerCommand } from '../../store/types'
import { normalizeEngineCapabilityStatus, normalizeEngineVersionStatus } from '../../engineCapabilities'
import { createConfigPreflightKey } from './configPreflight'

type CompatibilityInput = {
  local: InstanceConfig | null
  currentEngine: EngineInfo | null
  trustedEngineId: string
}

export function useEngineCompatibility({ local, currentEngine, trustedEngineId }: CompatibilityInput) {
  const generateCommand = useAppStore(state => state.generateCommand)
  const probeEngineCapabilities = useAppStore(state => state.probeEngineCapabilities)
  const [unsupportedEngineFlags, setUnsupportedEngineFlags] = useState<string[]>([])
  const [commandPreview, setCommandPreview] = useState<GeneratedServerCommand | null>(null)
  const [commandPreviewKey, setCommandPreviewKey] = useState<string | null>(null)
  const [previewingCommand, setPreviewingCommand] = useState(false)
  const [probingEngineCompatibility, setProbingEngineCompatibility] = useState(false)
  const [autoProbeFailedFor, setAutoProbeFailedFor] = useState<string | null>(null)
  const mountedRef = useRef(true)
  const probeTargetRef = useRef<string | null>(null)
  const probeInFlightRef = useRef<Set<string>>(new Set())
  const previewCacheRef = useRef<{ key: string; result: GeneratedServerCommand } | null>(null)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  const status = normalizeEngineCapabilityStatus(currentEngine?.capabilities)
  const versionStatus = normalizeEngineVersionStatus(currentEngine?.capabilities)
  const managedMode = local?.launch_mode !== 'manual'
  const isTrustedSelection = Boolean(currentEngine && currentEngine.id === trustedEngineId)
  const defaultsProbeRequired = (currentEngine?.capabilities?.reportedDefaultsVersion ?? 0) < 1
  const capabilityProbeRequired = managedMode && isTrustedSelection
    && (status === 'unprobed' || versionStatus === 'unprobed' || defaultsProbeRequired)
    && autoProbeFailedFor !== currentEngine?.id

  useEffect(() => {
    if (!currentEngine || !capabilityProbeRequired) {
      probeTargetRef.current = null
      setProbingEngineCompatibility(false)
      if (status !== 'unprobed' && versionStatus !== 'unprobed') setAutoProbeFailedFor(null)
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
  }, [capabilityProbeRequired, currentEngine, probeEngineCapabilities, status, versionStatus])

  useEffect(() => {
    if (!local || local.launch_mode === 'manual' || !currentEngine || status !== 'detected') {
      setUnsupportedEngineFlags([])
      setCommandPreview(null)
      setCommandPreviewKey(null)
      previewCacheRef.current = null
      setPreviewingCommand(false)
      return
    }
    let cancelled = false
    const previewKey = createConfigPreflightKey(local, currentEngine.exe)
    const cached = previewCacheRef.current
    if (cached?.key === previewKey) {
      setUnsupportedEngineFlags(cached.result.unsupportedFlags)
      setCommandPreview(cached.result)
      setCommandPreviewKey(previewKey)
      setPreviewingCommand(false)
      return
    }
    setCommandPreview(null)
    setCommandPreviewKey(null)
    setPreviewingCommand(true)
    const timer = setTimeout(() => {
      void generateCommand(local, currentEngine.exe)
        .then(result => {
          if (!cancelled) {
            setUnsupportedEngineFlags(result.unsupportedFlags)
            setCommandPreview(result)
            setCommandPreviewKey(previewKey)
            previewCacheRef.current = { key: previewKey, result }
          }
        })
        .catch(() => {
          if (!cancelled) {
            setUnsupportedEngineFlags([])
            setCommandPreview(null)
            setCommandPreviewKey(null)
            previewCacheRef.current = null
          }
        })
        .finally(() => {
          if (!cancelled) setPreviewingCommand(false)
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
    commandPreview,
    commandPreviewKey,
    previewingCommand,
    probingEngineCompatibility,
    capabilityProbeRequired,
  }
}
