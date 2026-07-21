import { useEffect, useMemo, useState } from 'react'
import { invokeApp as invoke } from '../lib/ipc'
import { Activity, AlertTriangle, Copy, HeartPulse, Plus, PowerOff, RefreshCw, Route, Save, Server, Square, Trash2, Zap } from 'lucide-react'
import { useAppStore } from '../store'
import { formatHostPort, httpUrl } from '../utils/network'
import { useI18n } from '../i18n'
import { getProxyLabels } from '../i18n/pageLabels'
import { Badge, Button, DataTable, EmptyPanel, IconButton, MetricCard, SelectInput, StatusBadge, Surface, TextInput } from './ui'

type ProxyRoute = {
  id: string
  enabled: boolean
  priority: number
  modelAlias: string
  targetInstanceId: string
}

type ProxyConfig = {
  enabled: boolean
  host: string
  port: number
  publicApiKey: string
  defaultInstanceId: string
  routingStrategy: string
  timeoutMs: number
  backgroundServiceMode: boolean
  runtimeServiceEnabled: boolean
  routes: ProxyRoute[]
}

type ProxyStatus = {
  running: boolean
  boundAddr: string
  activeRoutes: number
  lastError: string | null
}

type RuntimeServiceView = {
  servicePid: number
  serviceVersion: string
  backgroundEnabled: boolean
  registeredForLogin: boolean
  managedInstances: number
  lastError: string
}

type ProxyTarget = {
  instanceId: string
  name: string
  alias: string
  endpoint: string
  status: 'running' | 'stopped' | 'unknown'
  source: 'proxy' | 'instances'
}

type RouteIssue = {
  modelAlias?: string
  targetInstanceId?: string
}

type RouteAvailabilityKind = 'current' | 'standby' | 'stopped' | 'unknown' | 'missing' | 'disabled' | 'pending' | 'invalid'

type RouteAvailability = {
  kind: RouteAvailabilityKind
}

type RouteTestView = {
  tone: 'emerald' | 'amber' | 'red'
  message: string
}

const defaultConfig: ProxyConfig = {
  enabled: false,
  host: '127.0.0.1',
  port: 11435,
  publicApiKey: '',
  defaultInstanceId: '',
  routingStrategy: 'firstHealthy',
  timeoutMs: 600000,
  backgroundServiceMode: false,
  runtimeServiceEnabled: false,
  routes: [],
}

const defaultRuntimeService: RuntimeServiceView = {
  servicePid: 0,
  serviceVersion: '',
  backgroundEnabled: false,
  registeredForLogin: false,
  managedInstances: 0,
  lastError: '',
}

function getString(record: Record<string, unknown>, keys: string[], fallback = '') {
  for (const key of keys) {
    const value = record[key]
    if (typeof value === 'string') return value
    if (typeof value === 'number') return String(value)
  }
  return fallback
}

function getNumber(record: Record<string, unknown>, keys: string[], fallback = 0) {
  for (const key of keys) {
    const value = record[key]
    if (typeof value === 'number' && Number.isFinite(value)) return value
    if (typeof value === 'string') {
      const parsed = Number(value)
      if (Number.isFinite(parsed)) return parsed
    }
  }
  return fallback
}

function getBoolean(record: Record<string, unknown>, keys: string[], fallback = false) {
  for (const key of keys) {
    const value = record[key]
    if (typeof value === 'boolean') return value
  }
  return fallback
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}

function normalizeRoute(value: unknown, index: number): ProxyRoute {
  const record = asRecord(value)
  return {
    id: getString(record, ['id'], `route-${index + 1}`),
    enabled: getBoolean(record, ['enabled'], true),
    priority: getNumber(record, ['priority'], index + 1),
    modelAlias: getString(record, ['model_alias', 'modelAlias', 'model_pattern', 'modelPattern', 'model']),
    targetInstanceId: getString(record, ['target_instance_id', 'targetInstanceId', 'target_id', 'targetId', 'instance_id', 'instanceId']),
  }
}

function normalizeConfig(value: unknown): ProxyConfig {
  const record = asRecord(value)
  const routesValue = Array.isArray(record.routes) ? record.routes : []
  const routeIds = new Set<string>()
  const routes = routesValue.map(normalizeRoute).map((route, index) => {
    let id = route.id.trim()
    if (!id || routeIds.has(id)) {
      const base = `route-${index + 1}`
      id = base
      let suffix = 2
      while (routeIds.has(id)) {
        id = `${base}-${suffix}`
        suffix += 1
      }
    }
    routeIds.add(id)
    return { ...route, id }
  })
  return {
    enabled: getBoolean(record, ['enabled'], defaultConfig.enabled),
    host: getString(record, ['host', 'listen_host', 'listenHost'], defaultConfig.host),
    port: getNumber(record, ['port', 'listen_port', 'listenPort'], defaultConfig.port),
    publicApiKey: getString(record, ['public_api_key', 'publicApiKey']),
    defaultInstanceId: getString(record, ['default_instance_id', 'defaultInstanceId', 'default_target_id', 'defaultTargetId']),
    routingStrategy: getString(record, ['routing_strategy', 'routingStrategy'], defaultConfig.routingStrategy),
    timeoutMs: getNumber(record, ['timeout_ms', 'timeoutMs'], defaultConfig.timeoutMs),
    backgroundServiceMode: getBoolean(record, ['background_service_mode', 'backgroundServiceMode'], defaultConfig.backgroundServiceMode),
    runtimeServiceEnabled: getBoolean(record, ['runtime_service_enabled', 'runtimeServiceEnabled'], defaultConfig.runtimeServiceEnabled),
    routes,
  }
}

function normalizeStatus(value: unknown, config: ProxyConfig): ProxyStatus {
  const record = asRecord(value)
  return {
    running: getBoolean(record, ['running', 'is_running', 'isRunning'], false),
    boundAddr: getString(record, ['bound_addr', 'boundAddr', 'endpoint', 'url'], formatHostPort(config.host, config.port)),
    activeRoutes: getNumber(record, ['active_routes', 'activeRoutes'], config.routes.filter(route => route.enabled).length),
    lastError: getString(record, ['last_error', 'lastError', 'error']) || null,
  }
}

function normalizeRuntimeService(value: unknown): RuntimeServiceView {
  const record = asRecord(value)
  const running = asRecord(record.running)
  return {
    servicePid: getNumber(record, ['servicePid', 'service_pid']),
    serviceVersion: getString(record, ['serviceVersion', 'service_version']),
    backgroundEnabled: getBoolean(record, ['backgroundEnabled', 'background_enabled']),
    registeredForLogin: getBoolean(record, ['registeredForLogin', 'registered_for_login']),
    managedInstances: Object.keys(running).length,
    lastError: getString(record, ['lastError', 'last_error']),
  }
}

function normalizeTarget(value: unknown, index: number): ProxyTarget {
  const record = asRecord(value)
  const host = getString(record, ['host'], '127.0.0.1')
  const port = getNumber(record, ['port'], 0)
  const endpoint = getString(record, ['endpoint', 'url'], port > 0 ? httpUrl(host, port) : '')
  const rawStatus = getString(record, ['status'], 'unknown').toLowerCase()
  const running = record.running
  const status: ProxyTarget['status'] = typeof running === 'boolean'
    ? running ? 'running' : 'stopped'
    : rawStatus === 'running' || rawStatus === 'online'
      ? 'running'
      : rawStatus === 'stopped' || rawStatus === 'offline'
        ? 'stopped'
        : 'unknown'

  return {
    instanceId: getString(record, ['instance_id', 'instanceId', 'id'], `target-${index + 1}`),
    name: getString(record, ['name'], `Target ${index + 1}`),
    alias: getString(record, ['alias']),
    endpoint,
    status,
    source: 'proxy',
  }
}

function toCommandConfig(config: ProxyConfig) {
  return {
    enabled: config.enabled,
    host: config.host,
    port: config.port,
    public_api_key: config.publicApiKey,
    default_instance_id: config.defaultInstanceId,
    routing_strategy: config.routingStrategy,
    timeout_ms: config.timeoutMs,
    background_service_mode: config.backgroundServiceMode,
    runtime_service_enabled: config.runtimeServiceEnabled,
    routes: config.routes.map(route => ({
      id: route.id,
      enabled: route.enabled,
      priority: route.priority,
      model_alias: route.modelAlias.trim(),
      target_instance_id: route.targetInstanceId.trim(),
    })),
  }
}

function errorMessage(error: unknown) {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return String(error)
}

function endpointUrl(boundAddr: string, config: ProxyConfig) {
  const value = boundAddr || formatHostPort(config.host, config.port)
  return value.startsWith('http://') || value.startsWith('https://') ? value : `http://${value}`
}

function sameRoute(left: ProxyRoute, right: ProxyRoute) {
  return left.id === right.id
    && left.enabled === right.enabled
    && left.priority === right.priority
    && left.modelAlias.trim() === right.modelAlias.trim()
    && left.targetInstanceId.trim() === right.targetInstanceId.trim()
}

function routeAvailabilityView(kind: RouteAvailabilityKind, labels: ReturnType<typeof getProxyLabels>) {
  switch (kind) {
    case 'current': return { label: labels.routeCurrent, tone: 'emerald' as const }
    case 'standby': return { label: labels.routeStandby, tone: 'blue' as const }
    case 'stopped': return { label: labels.routeTargetStopped, tone: 'red' as const }
    case 'unknown': return { label: labels.routeTargetUnknown, tone: 'amber' as const }
    case 'missing': return { label: labels.routeTargetMissingShort, tone: 'red' as const }
    case 'disabled': return { label: labels.disabled, tone: 'slate' as const }
    case 'pending': return { label: labels.routePendingSave, tone: 'amber' as const }
    case 'invalid': return { label: labels.routeIncomplete, tone: 'red' as const }
  }
}

export default function ProxyPage() {
  const { lang } = useI18n()
  const instances = useAppStore(state => state.instances)
  const [config, setConfig] = useState<ProxyConfig>(defaultConfig)
  const [draft, setDraft] = useState<ProxyConfig>(defaultConfig)
  const [status, setStatus] = useState<ProxyStatus>(normalizeStatus(null, defaultConfig))
  const [statusFresh, setStatusFresh] = useState(false)
  const [runtimeService, setRuntimeService] = useState<RuntimeServiceView>(defaultRuntimeService)
  const [runtimeFresh, setRuntimeFresh] = useState(false)
  const [targets, setTargets] = useState<ProxyTarget[]>([])
  const [targetsFresh, setTargetsFresh] = useState(false)
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [busyAction, setBusyAction] = useState<'start' | 'stop' | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [commandsReady, setCommandsReady] = useState(true)
  const [testingRouteId, setTestingRouteId] = useState<string | null>(null)
  const [routeTests, setRouteTests] = useState<Record<string, RouteTestView>>({})
  const [stopRuntimeConfirmOpen, setStopRuntimeConfirmOpen] = useState(false)
  const [stoppingRuntime, setStoppingRuntime] = useState(false)

  const labels = useMemo(() => getProxyLabels(lang), [lang])

  const isLocalHost = (host: string) => ['', 'localhost', '127.0.0.1', '::1', '[::1]'].includes(host.trim())
  const requiresPublicKey = !isLocalHost(draft.host) && !draft.publicApiKey.trim()

  const fallbackTargets = useMemo<ProxyTarget[]>(() => instances.map(instance => ({
    instanceId: instance.id,
    name: instance.name,
    alias: instance.config.alias,
    endpoint: httpUrl(instance.config.host, instance.config.port),
    status: instance.status === 'running' ? 'running' : instance.status === 'stopped' ? 'stopped' : 'unknown',
    source: 'instances',
  })), [instances])

  const displayedTargets = targetsFresh ? targets : targets.length > 0 ? targets : fallbackTargets
  const effectiveTargets = targetsFresh
    ? displayedTargets
    : displayedTargets.map(target => ({ ...target, status: 'unknown' as const }))
  const selectedTarget = effectiveTargets.find(target => target.instanceId === draft.defaultInstanceId)
  const endpoint = endpointUrl(status.boundAddr, draft)
  const routeIssues = useMemo(() => {
    const knownTargetIds = new Set(effectiveTargets.map(target => target.instanceId))
    const issues = new Map<string, RouteIssue>()
    for (const route of draft.routes) {
      if (!route.enabled) continue
      const issue: RouteIssue = {}
      if (!route.modelAlias.trim()) issue.modelAlias = labels.routeModelRequired
      if (!route.targetInstanceId.trim()) issue.targetInstanceId = labels.routeTargetRequired
      else if (!knownTargetIds.has(route.targetInstanceId.trim())) issue.targetInstanceId = labels.routeTargetMissing
      if (issue.modelAlias || issue.targetInstanceId) issues.set(route.id, issue)
    }
    return issues
  }, [draft.routes, effectiveTargets, labels])
  const hasRouteIssues = routeIssues.size > 0
  const routeAvailability = useMemo(() => {
    const targetById = new Map(effectiveTargets.map(target => [target.instanceId, target]))
    const savedById = new Map(config.routes.map(route => [route.id, route]))
    const candidatesByModel = new Map<string, Array<{ route: ProxyRoute; index: number }>>()
    let healthyRoutes = 0

    config.routes.forEach((route, index) => {
      if (!route.enabled || !route.modelAlias.trim() || !route.targetInstanceId.trim()) return
      const target = targetById.get(route.targetInstanceId.trim())
      if (target?.status !== 'running') return
      healthyRoutes += 1
      const model = route.modelAlias.trim()
      const candidates = candidatesByModel.get(model) ?? []
      candidates.push({ route, index })
      candidatesByModel.set(model, candidates)
    })

    const selectedRouteIds = new Set<string>()
    for (const candidates of candidatesByModel.values()) {
      candidates.sort((left, right) => left.route.priority - right.route.priority || left.index - right.index)
      if (candidates[0]) selectedRouteIds.add(candidates[0].route.id)
    }

    const byId = new Map<string, RouteAvailability>()
    for (const route of draft.routes) {
      const saved = savedById.get(route.id)
      if (!saved || !sameRoute(route, saved)) {
        byId.set(route.id, { kind: 'pending' })
        continue
      }
      if (!saved.enabled) {
        byId.set(route.id, { kind: 'disabled' })
        continue
      }
      if (!saved.modelAlias.trim() || !saved.targetInstanceId.trim()) {
        byId.set(route.id, { kind: 'invalid' })
        continue
      }
      const target = targetById.get(saved.targetInstanceId.trim()) ?? null
      if (!target) {
        byId.set(route.id, { kind: 'missing' })
      } else if (target.status === 'stopped') {
        byId.set(route.id, { kind: 'stopped' })
      } else if (target.status !== 'running') {
        byId.set(route.id, { kind: 'unknown' })
      } else if (selectedRouteIds.has(route.id)) {
        byId.set(route.id, { kind: 'current' })
      } else {
        byId.set(route.id, { kind: 'standby' })
      }
    }

    return { byId, healthyRoutes }
  }, [config.routes, draft.routes, effectiveTargets])

  const loadProxy = async () => {
    setLoading(true)
    setError('')
    setNotice('')
    setRouteTests({})

    try {
      const configResult = await invoke<unknown>('get_proxy_config')
      const nextConfig = normalizeConfig(configResult)
      setConfig(nextConfig)
      setDraft(nextConfig)
      setCommandsReady(true)

      const [statusResult, targetsResult, runtimeResult] = await Promise.allSettled([
        invoke<unknown>('get_proxy_status'),
        invoke<unknown[]>('list_proxy_targets'),
        invoke<unknown>('get_runtime_service_status'),
      ])

      if (statusResult.status === 'fulfilled') {
        setStatus(normalizeStatus(statusResult.value, nextConfig))
        setStatusFresh(true)
      } else {
        setStatus(normalizeStatus(null, nextConfig))
        setStatusFresh(false)
      }

      if (targetsResult.status === 'fulfilled' && Array.isArray(targetsResult.value)) {
        setTargets(targetsResult.value.map(normalizeTarget))
        setTargetsFresh(true)
      } else {
        setTargets([])
        setTargetsFresh(false)
      }
      if (runtimeResult.status === 'fulfilled') {
        setRuntimeService(normalizeRuntimeService(runtimeResult.value))
        setRuntimeFresh(true)
      } else {
        setRuntimeService(defaultRuntimeService)
        setRuntimeFresh(false)
      }
    } catch (loadError) {
      setCommandsReady(false)
      setTargets([])
      setTargetsFresh(false)
      setConfig(defaultConfig)
      setDraft(defaultConfig)
      setStatus(normalizeStatus(null, defaultConfig))
      setStatusFresh(false)
      setRuntimeFresh(false)
      setError(errorMessage(loadError))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void loadProxy()
  }, [])

  useEffect(() => {
    let cancelled = false
    const refreshLiveState = async () => {
      const [statusResult, targetsResult, runtimeResult] = await Promise.allSettled([
        invoke<unknown>('get_proxy_status'),
        invoke<unknown[]>('list_proxy_targets'),
        invoke<unknown>('get_runtime_service_status'),
      ])
      if (cancelled) return
      if (statusResult.status === 'fulfilled') {
        setStatus(normalizeStatus(statusResult.value, config))
        setStatusFresh(true)
      } else {
        setStatusFresh(false)
      }
      if (targetsResult.status === 'fulfilled' && Array.isArray(targetsResult.value)) {
        setTargets(targetsResult.value.map(normalizeTarget))
        setTargetsFresh(true)
      } else {
        setTargetsFresh(false)
      }
      if (runtimeResult.status === 'fulfilled') {
        setRuntimeService(normalizeRuntimeService(runtimeResult.value))
        setRuntimeFresh(true)
      } else {
        setRuntimeFresh(false)
      }
    }
    const timer = window.setInterval(() => {
      void refreshLiveState()
    }, 5000)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [config])

  const updateDraft = (patch: Partial<ProxyConfig>) => {
    setDraft(current => ({ ...current, ...patch }))
  }

  const updateRoute = (id: string, patch: Partial<ProxyRoute>) => {
    setRouteTests(current => {
      const next = { ...current }
      delete next[id]
      return next
    })
    setDraft(current => ({
      ...current,
      routes: current.routes.map(route => route.id === id ? { ...route, ...patch } : route),
    }))
  }

  const addRoute = () => {
    setDraft(current => ({
      ...current,
      routes: [
        ...current.routes,
        {
          id: crypto.randomUUID(),
          enabled: true,
          priority: Math.max(0, ...current.routes.map(route => route.priority)) + 1,
          modelAlias: '',
          targetInstanceId: current.defaultInstanceId || (effectiveTargets.length === 1 ? effectiveTargets[0].instanceId : ''),
        },
      ],
    }))
  }

  const removeRoute = (id: string) => {
    setRouteTests(current => {
      const next = { ...current }
      delete next[id]
      return next
    })
    setDraft(current => ({
      ...current,
      routes: current.routes.filter(route => route.id !== id),
    }))
  }

  const testRoute = async (route: ProxyRoute) => {
    const saved = config.routes.find(candidate => candidate.id === route.id)
    if (!saved || !sameRoute(route, saved)) {
      setRouteTests(current => ({
        ...current,
        [route.id]: { tone: 'amber', message: labels.routeTestSaveFirst },
      }))
      return
    }

    setTestingRouteId(route.id)
    setRouteTests(current => {
      const next = { ...current }
      delete next[route.id]
      return next
    })
    try {
      const resolvedValue = await invoke<unknown>('test_proxy_route', { model: saved.modelAlias.trim() })
      const resolved = normalizeTarget(resolvedValue, 0)
      const selected = resolved.instanceId === saved.targetInstanceId.trim()
      setRouteTests(current => ({
        ...current,
        [route.id]: {
          tone: selected ? 'emerald' : 'amber',
          message: `${selected ? labels.routeTestHit : labels.routeTestRerouted}: ${resolved.name}`,
        },
      }))
    } catch (testError) {
      setRouteTests(current => ({
        ...current,
        [route.id]: { tone: 'red', message: `${labels.routeTestFailed}: ${errorMessage(testError)}` },
      }))
    } finally {
      setTestingRouteId(null)
    }
  }

  const saveConfig = async () => {
    if (hasRouteIssues) {
      setError(labels.routeValidationSummary)
      return
    }
    setSaving(true)
    setError('')
    setNotice('')

    try {
      const savedValue = await invoke<unknown>('save_proxy_config', { config: toCommandConfig(draft) })
      const nextConfig = normalizeConfig(savedValue ?? toCommandConfig(draft))
      const nextStatusValue = await invoke<unknown>('get_proxy_status').catch(() => null)
      setConfig(nextConfig)
      setDraft(nextConfig)
      setRouteTests({})
      setStatus(nextStatusValue == null
        ? { ...status, activeRoutes: nextConfig.routes.filter(route => route.enabled).length }
        : normalizeStatus(nextStatusValue, nextConfig))
      setStatusFresh(nextStatusValue != null)
      setNotice(labels.saved)
      setCommandsReady(true)
    } catch (saveError) {
      setCommandsReady(true)
      setError(errorMessage(saveError))
    } finally {
      setSaving(false)
    }
  }

  const persistDraftConfig = async () => {
    const savedValue = await invoke<unknown>('save_proxy_config', { config: toCommandConfig(draft) })
    const nextConfig = normalizeConfig(savedValue ?? toCommandConfig(draft))
    setConfig(nextConfig)
    setDraft(nextConfig)
    setRouteTests({})
    setCommandsReady(true)
    return nextConfig
  }

  const setProxyRunning = async (action: 'start' | 'stop') => {
    setBusyAction(action)
    setError('')
    setNotice('')

    try {
      let effectiveConfig = draft
      if (action === 'start' && dirty) {
        effectiveConfig = await persistDraftConfig()
      }
      const actionStatus = await invoke<unknown>(action === 'start' ? 'start_proxy' : 'stop_proxy')
      const nextStatus = await invoke<unknown>('get_proxy_status').catch(() => actionStatus)
      setStatus(normalizeStatus(nextStatus, effectiveConfig))
      setStatusFresh(nextStatus != null)
      const enabled = action === 'start'
      setConfig(current => ({ ...current, enabled }))
      setDraft(current => ({ ...current, enabled }))
      setCommandsReady(true)
    } catch (actionError) {
      setCommandsReady(true)
      setError(errorMessage(actionError))
    } finally {
      setBusyAction(null)
    }
  }

  const copyEndpoint = async () => {
    try {
      await navigator.clipboard.writeText(endpoint)
      setNotice(labels.copied)
    } catch {
      // ignore clipboard failures
    }
  }

  const stopBackgroundRuntime = async () => {
    setStoppingRuntime(true)
    setError('')
    setNotice('')
    try {
      await invoke('stop_background_runtime')
      setStopRuntimeConfirmOpen(false)
      await loadProxy()
      setNotice(labels.backgroundStopped)
    } catch (stopError) {
      const message = errorMessage(stopError)
      setStopRuntimeConfirmOpen(false)
      await loadProxy()
      setError(message)
    } finally {
      setStoppingRuntime(false)
    }
  }

  const dirty = JSON.stringify(config) !== JSON.stringify(draft)

  return (
    <div className="space-y-5">
      <Surface as="section" className="p-5" data-guide="proxy-overview">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
          <div className="min-w-0">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <h2 className="text-2xl font-semibold text-slate-950 dark:text-slate-50">{labels.title}</h2>
              <StatusBadge tone={!statusFresh ? 'amber' : status.running ? 'emerald' : 'slate'}>
                {!statusFresh ? labels.unknown : status.running ? labels.running : labels.stopped}
              </StatusBadge>
              {!commandsReady ? <Badge tone="amber">{labels.unavailable}</Badge> : null}
              {dirty ? <Badge tone="blue">{labels.unsaved}</Badge> : null}
            </div>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.subtitle}</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button onClick={loadProxy} disabled={loading} icon={<RefreshCw className="h-4 w-4" />}>
              {labels.refresh}
            </Button>
            <Button onClick={saveConfig} disabled={saving || hasRouteIssues} variant="primary" icon={<Save className="h-4 w-4" />}>
              {labels.save}
            </Button>
            {statusFresh && status.running ? (
              <Button onClick={() => setProxyRunning('stop')} disabled={busyAction !== null} variant="danger" icon={<Square className="h-4 w-4" />}>
                {labels.stop}
              </Button>
            ) : (
              <Button onClick={() => setProxyRunning('start')} disabled={!statusFresh || busyAction !== null || requiresPublicKey || hasRouteIssues} variant="success" icon={<Zap className="h-4 w-4" />}>
                {labels.start}
              </Button>
            )}
          </div>
        </div>

        {(!commandsReady || error || notice) ? (
          <div className="mt-4 space-y-2">
            {!commandsReady ? (
              <div className="rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-700 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
                {labels.notReady}
              </div>
            ) : null}
            {error ? (
              <div className="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200">
                {error}
              </div>
            ) : null}
            {notice ? (
              <div className="rounded-lg border border-emerald-200 bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:border-emerald-500/20 dark:bg-emerald-500/10 dark:text-emerald-200">
                {notice}
              </div>
            ) : null}
          </div>
        ) : null}
      </Surface>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-5">
        <MetricCard label={labels.endpoint} value={endpoint} valueClassName="text-base" icon={<Activity className="h-5 w-5" />} />
        <MetricCard label={labels.defaultTarget} value={selectedTarget?.name || labels.noDefault} valueClassName="text-base" icon={<Server className="h-5 w-5" />} />
        <MetricCard label={labels.connections} value={targetsFresh ? effectiveTargets.length : '—'} icon={<Zap className="h-5 w-5" />} />
        <MetricCard label={labels.requests} value={config.routes.filter(route => route.enabled).length} icon={<Route className="h-5 w-5" />} />
        <MetricCard label={labels.healthyRoutes} value={routeAvailability.healthyRoutes} icon={<HeartPulse className="h-5 w-5" />} />
      </div>

      <div className="grid gap-5 2xl:grid-cols-[minmax(0,1fr)_360px]">
        <div className="min-w-0 space-y-5">
          <Surface as="section" className="p-5">
            <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
              <div>
                <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.listen}</h3>
                <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">{endpoint}</p>
              </div>
              <Button onClick={copyEndpoint} icon={<Copy className="h-4 w-4" />}>
                {labels.endpoint}
              </Button>
            </div>

            <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_140px_minmax(0,1fr)]">
              <label className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.host}</span>
                <TextInput value={draft.host} onChange={event => updateDraft({ host: event.target.value })} />
              </label>
              <label className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.port}</span>
                <TextInput
                  type="number"
                  min={1}
                  max={65535}
                  value={draft.port}
                  onChange={event => updateDraft({ port: Math.max(1, Math.min(65535, Number(event.target.value) || defaultConfig.port)) })}
                />
              </label>
              <label className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.defaultTarget}</span>
                <SelectInput value={draft.defaultInstanceId} onChange={event => updateDraft({ defaultInstanceId: event.target.value })} className="w-full">
                  <option value="">{labels.noDefault}</option>
                  {effectiveTargets.map(target => (
                    <option key={target.instanceId} value={target.instanceId}>
                      {target.name} · {target.status === 'running' ? labels.running : target.status === 'stopped' ? labels.stopped : labels.unknown}
                    </option>
                  ))}
                </SelectInput>
                <span className="mt-1.5 block text-xs leading-5 text-slate-500 dark:text-slate-400">{labels.defaultTargetHint}</span>
              </label>
            </div>

            <div className="mt-3 grid gap-3 md:grid-cols-[minmax(0,1fr)_180px]">
              <label className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.publicKey}</span>
                <TextInput type="password" autoComplete="off" value={draft.publicApiKey} onChange={event => updateDraft({ publicApiKey: event.target.value })} />
                {requiresPublicKey ? (
                  <div className="mt-2 flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs leading-5 text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200">
                    <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                    <span>{labels.publicKeyRequired}</span>
                  </div>
                ) : null}
              </label>
              <label className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.timeout}</span>
                <TextInput
                  type="number"
                  min={1000}
                  value={draft.timeoutMs}
                  onChange={event => updateDraft({ timeoutMs: Math.max(1000, Number(event.target.value) || defaultConfig.timeoutMs) })}
                />
              </label>
            </div>

            {status.lastError ? (
              <div className="mt-4 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200">
                <span className="font-semibold">{labels.lastError}: </span>{status.lastError}
              </div>
            ) : null}
          </Surface>

          <Surface as="section" className="p-5">
            <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.routeTable}</h3>
                <p className="mt-1 text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.routeTableHint}</p>
              </div>
              <Button onClick={addRoute} variant="primary" className="shrink-0 whitespace-nowrap" icon={<Plus className="h-4 w-4" />}>
                {labels.addRoute}
              </Button>
            </div>

            <DataTable
              density="compact"
              rows={draft.routes}
              getRowKey={route => route.id}
              empty={<EmptyPanel title={labels.noRoutes} />}
              columns={[
                {
                  key: 'enabled',
                  header: labels.status,
                  width: 150,
                  render: route => (
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        role="switch"
                        aria-checked={route.enabled}
                        aria-label={labels.routeStatusControl}
                        title={labels.routeStatusControl}
                        onClick={() => updateRoute(route.id, { enabled: !route.enabled })}
                        className={`relative inline-flex h-6 w-11 shrink-0 rounded-full transition ${route.enabled ? 'bg-emerald-600' : 'bg-slate-300 dark:bg-slate-700'}`}
                      >
                        <span className={`absolute top-1 h-4 w-4 rounded-full bg-white shadow-sm transition ${route.enabled ? 'left-6' : 'left-1'}`} />
                      </button>
                      <span className={`text-xs font-medium ${route.enabled ? 'text-emerald-700 dark:text-emerald-300' : 'text-slate-500 dark:text-slate-400'}`}>
                        {route.enabled ? labels.enabled : labels.disabled}
                      </span>
                    </div>
                  ),
                },
                {
                  key: 'priority',
                  header: labels.priority,
                  width: 96,
                  render: route => (
                    <TextInput
                      type="number"
                      aria-label={labels.priority}
                      value={route.priority}
                      onChange={event => updateRoute(route.id, { priority: Number(event.target.value) || 0 })}
                      className="h-9"
                    />
                  ),
                },
                {
                  key: 'model',
                  header: labels.modelPattern,
                  minWidth: 220,
                  render: route => {
                    const issue = routeIssues.get(route.id)?.modelAlias
                    return (
                      <div>
                        <TextInput
                          aria-label={labels.modelPattern}
                          aria-invalid={Boolean(issue)}
                          value={route.modelAlias}
                          onChange={event => updateRoute(route.id, { modelAlias: event.target.value })}
                          className={`h-9 ${issue ? 'border-rose-400 dark:border-rose-500' : ''}`}
                        />
                        {issue ? <p className="mt-1 text-xs text-rose-600 dark:text-rose-400" role="alert">{issue}</p> : null}
                      </div>
                    )
                  },
                },
                {
                  key: 'target',
                  header: labels.target,
                  minWidth: 220,
                  render: route => {
                    const issue = routeIssues.get(route.id)?.targetInstanceId
                    return (
                      <div>
                        <SelectInput
                          aria-label={labels.target}
                          aria-invalid={Boolean(issue)}
                          value={route.targetInstanceId}
                          onChange={event => updateRoute(route.id, { targetInstanceId: event.target.value })}
                          className={`h-9 w-full ${issue ? 'border-rose-400 dark:border-rose-500' : ''}`}
                        >
                          <option value="">{labels.selectTarget}</option>
                          {effectiveTargets.map(target => (
                            <option key={target.instanceId} value={target.instanceId}>
                              {target.name} · {target.status === 'running' ? labels.running : target.status === 'stopped' ? labels.stopped : labels.unknown}
                            </option>
                          ))}
                        </SelectInput>
                        {issue ? <p className="mt-1 text-xs text-rose-600 dark:text-rose-400" role="alert">{issue}</p> : null}
                      </div>
                    )
                  },
                },
                {
                  key: 'availability',
                  header: labels.routeAvailability,
                  minWidth: 180,
                  render: route => {
                    const availability = routeAvailability.byId.get(route.id) ?? { kind: 'pending' as const }
                    const view = routeAvailabilityView(availability.kind, labels)
                    const test = routeTests[route.id]
                    return (
                      <div className="min-w-0">
                        <StatusBadge tone={view.tone}>{view.label}</StatusBadge>
                        {test ? (
                          <p role={test.tone === 'red' ? 'alert' : 'status'} className={`mt-1.5 max-w-[240px] text-xs leading-5 ${
                            test.tone === 'emerald'
                              ? 'text-emerald-700 dark:text-emerald-300'
                              : test.tone === 'amber'
                                ? 'text-amber-700 dark:text-amber-300'
                                : 'text-rose-700 dark:text-rose-300'
                          }`}>
                            {test.message}
                          </p>
                        ) : null}
                      </div>
                    )
                  },
                },
                {
                  key: 'actions',
                  header: labels.actions,
                  width: 176,
                  align: 'right',
                  render: route => {
                    const availability = routeAvailability.byId.get(route.id)?.kind ?? 'pending'
                    const canTest = !['disabled', 'pending', 'invalid', 'missing'].includes(availability)
                    return (
                      <div className="flex items-center justify-end gap-1.5">
                        <Button
                          size="sm"
                          disabled={!canTest || testingRouteId !== null}
                          title={canTest ? labels.routeTestHint : labels.routeTestSaveFirst}
                          onClick={() => void testRoute(route)}
                          icon={<Route className="h-3.5 w-3.5" />}
                        >
                          {testingRouteId === route.id ? labels.testingRoute : labels.testRoute}
                        </Button>
                        <IconButton
                          label={labels.removeRoute}
                          onClick={() => removeRoute(route.id)}
                          icon={<Trash2 className="h-4 w-4" />}
                        />
                      </div>
                    )
                  },
                },
              ]}
            />
            {hasRouteIssues ? (
              <div className="mt-3 flex items-start gap-2 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs leading-5 text-rose-700 dark:border-rose-500/20 dark:bg-rose-500/10 dark:text-rose-300">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <span>{labels.routeValidationSummary}</span>
              </div>
            ) : null}
            <div className="mt-3 flex items-start gap-2 rounded-lg border border-blue-200 bg-blue-50 px-3 py-2 text-xs leading-5 text-blue-800 dark:border-blue-500/20 dark:bg-blue-500/10 dark:text-blue-200">
              <Route className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{labels.routeIdentityHint} {labels.routeHealthHint}</span>
            </div>
          </Surface>
        </div>

        <div className="min-w-0 space-y-5">
          <Surface as="section" className="p-5">
            <div className="mb-4 flex items-start justify-between gap-3">
              <div>
                <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.targetList}</h3>
                <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">
                  {targetsFresh ? `${targets.length}` : labels.unavailable}
                </p>
              </div>
              {!targetsFresh ? <AlertTriangle className="h-5 w-5 shrink-0 text-amber-500" /> : null}
            </div>

            <div className="space-y-2">
              {effectiveTargets.length === 0 ? (
                <EmptyPanel title={labels.noTargets} />
              ) : effectiveTargets.map(target => (
                <div key={target.instanceId} className="min-w-0 rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-950/70">
                  <div className="flex min-w-0 items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="truncate text-sm font-semibold text-slate-950 dark:text-slate-50" title={target.name}>{target.name}</div>
                      <div className="mt-1 truncate font-mono text-xs text-slate-500 dark:text-slate-400" title={target.endpoint}>{target.endpoint}</div>
                    </div>
                    <StatusBadge tone={target.status === 'running' ? 'emerald' : target.status === 'stopped' ? 'slate' : 'amber'}>
                      {target.status === 'running' ? labels.running : target.status === 'stopped' ? labels.stopped : labels.unknown}
                    </StatusBadge>
                  </div>
                  {target.alias ? <div className="mt-2 truncate text-xs text-slate-500 dark:text-slate-400" title={target.alias}>{labels.alias}: {target.alias}</div> : null}
                </div>
              ))}
            </div>
          </Surface>

          <Surface as="section" className="p-5">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0">
                <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.keepAliveTitle}</h3>
                <p className="mt-2 text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.keepAliveDesc}</p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={draft.runtimeServiceEnabled}
                onClick={() => updateDraft({ runtimeServiceEnabled: !draft.runtimeServiceEnabled })}
                className={`relative mt-1 inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition ${
                  draft.runtimeServiceEnabled
                    ? 'border-blue-500 bg-blue-600'
                    : 'border-slate-300 bg-slate-200 dark:border-slate-700 dark:bg-slate-800'
                }`}
              >
                <span
                  className={`inline-block h-5 w-5 rounded-full bg-white shadow transition ${
                    draft.runtimeServiceEnabled ? 'translate-x-6' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>
            <div className="mt-4 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-xs font-medium text-slate-600 dark:border-slate-800 dark:bg-slate-950/70 dark:text-slate-300">
              {draft.runtimeServiceEnabled ? labels.enabled : labels.disabled}
            </div>
            <div className="mt-3 grid gap-2 sm:grid-cols-3">
              <div className="rounded-lg border border-slate-200 px-3 py-2 dark:border-slate-800">
                <div className="text-[11px] text-slate-500 dark:text-slate-400">{labels.runtimeProcess}</div>
                <div className="mt-1 truncate text-xs font-semibold text-slate-800 dark:text-slate-200" title={runtimeService.serviceVersion}>
                  {runtimeFresh && runtimeService.servicePid > 0 ? `PID ${runtimeService.servicePid}` : labels.unavailable}
                </div>
              </div>
              <div className="rounded-lg border border-slate-200 px-3 py-2 dark:border-slate-800">
                <div className="text-[11px] text-slate-500 dark:text-slate-400">{labels.loginRecovery}</div>
                <div className="mt-1 text-xs font-semibold text-slate-800 dark:text-slate-200">
                  {!runtimeFresh ? labels.unknown : runtimeService.registeredForLogin ? labels.registered : labels.notRegistered}
                </div>
              </div>
              <div className="rounded-lg border border-slate-200 px-3 py-2 dark:border-slate-800">
                <div className="text-[11px] text-slate-500 dark:text-slate-400">{labels.managedInstances}</div>
                <div className="mt-1 text-xs font-semibold text-slate-800 dark:text-slate-200">{runtimeFresh ? runtimeService.managedInstances : '—'}</div>
              </div>
            </div>
            {draft.runtimeServiceEnabled !== config.runtimeServiceEnabled
              || config.runtimeServiceEnabled !== runtimeService.backgroundEnabled ? (
              <p className="mt-3 text-xs text-amber-600 dark:text-amber-400">{labels.runtimeSyncPending}</p>
            ) : null}
            {runtimeService.lastError ? (
              <p className="mt-3 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-700 dark:border-rose-900/60 dark:bg-rose-950/30 dark:text-rose-300">
                {labels.runtimeLastError}: {runtimeService.lastError}
              </p>
            ) : null}
            {(config.runtimeServiceEnabled
              || runtimeService.backgroundEnabled
              || runtimeService.managedInstances > 0
              || status.running) ? (
              <div className="mt-4 flex flex-col gap-3 border-t border-slate-200 pt-4 dark:border-slate-800">
                <div>
                  <div className="text-sm font-semibold text-slate-900 dark:text-slate-100">{labels.stopRuntimeTitle}</div>
                  <p className="mt-1 text-xs leading-5 text-slate-500 dark:text-slate-400">{labels.stopRuntimeDesc}</p>
                </div>
                <Button
                  variant="danger"
                  icon={<PowerOff className="h-4 w-4" />}
                  disabled={stoppingRuntime}
                  onClick={() => setStopRuntimeConfirmOpen(true)}
                >
                  {labels.stopRuntimeAction}
                </Button>
              </div>
            ) : null}
          </Surface>
        </div>
      </div>
      {stopRuntimeConfirmOpen ? (
        <div className="fixed inset-0 z-[120] flex items-center justify-center bg-slate-950/60 px-4 backdrop-blur-sm" role="presentation">
          <div className="w-full max-w-lg rounded-xl border border-slate-200 bg-white p-6 shadow-2xl dark:border-slate-800 dark:bg-slate-950" role="alertdialog" aria-modal="true" aria-labelledby="stop-background-runtime-title">
            <h2 id="stop-background-runtime-title" className="text-xl font-semibold text-slate-950 dark:text-slate-50">
              {labels.stopRuntimeConfirmTitle}
            </h2>
            <p className="mt-3 text-sm leading-6 text-slate-600 dark:text-slate-300">
              {labels.stopRuntimeConfirmDesc}
            </p>
            <div className="mt-6 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
              <Button disabled={stoppingRuntime} onClick={() => setStopRuntimeConfirmOpen(false)}>
                {labels.cancel}
              </Button>
              <Button disabled={stoppingRuntime} variant="danger" onClick={() => void stopBackgroundRuntime()}>
                {stoppingRuntime ? labels.stoppingRuntime : labels.stopRuntimeConfirmAction}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  )
}
