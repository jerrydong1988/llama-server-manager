import { useEffect, useMemo, useRef, useState } from 'react'
import { invokeApp as invoke } from '../lib/ipc'
import { Activity, AlertTriangle, Copy, Eye, EyeOff, HeartPulse, Plus, PowerOff, RefreshCw, Route, Save, Server, Square, Trash2, Zap } from 'lucide-react'
import { useAppStore } from '../store'
import { formatHostPort, httpUrl } from '../utils/network'
import { useI18n } from '../i18n'
import { getProxyLabels } from '../i18n/pageLabels'
import { Badge, Button, DataTable, EmptyPanel, IconButton, MetricCard, SelectInput, StatusBadge, Surface, TextInput } from './ui'
import { CanaryRolloutPanel } from './CanaryRollout/CanaryRolloutPanel'

type ProxyRoute = {
  id: string
  enabled: boolean
  priority: number
  weight: number
  maxConcurrentRequests: number
  modelAlias: string
  targetInstanceId: string
}

type ProxyApiKey = {
  id: string
  name: string
  key: string
  enabled: boolean
  scopes: string[]
  requestsPerMinute: number
}

type ProxyConfig = {
  enabled: boolean
  host: string
  port: number
  legacyPublicApiKey: string
  defaultInstanceId: string
  routingStrategy: string
  strictModelRouting: boolean
  localityRoutingEnabled: boolean
  localityTtlMs: number
  localityMaxEntries: number
  connectTimeoutMs: number
  timeoutMs: number
  streamingIdleTimeoutMs: number
  healthCheckIntervalMs: number
  healthCheckTimeoutMs: number
  unhealthyThreshold: number
  recoveryCooldownMs: number
  maxConcurrentRequests: number
  queueTimeoutMs: number
  requestsPerMinute: number
  corsAllowedOrigins: string[]
  apiKeys: ProxyApiKey[]
  backgroundServiceMode: boolean
  runtimeServiceEnabled: boolean
  routes: ProxyRoute[]
}

type ProxyStatus = {
  running: boolean
  boundAddr: string
  activeRoutes: number
  healthyRoutes: number
  unhealthyRoutes: number
  inFlightRequests: number
  totalRequests: number
  operational: ProxyOperationalSnapshot
  lastError: string | null
}

type ProxyOperationalAlert = {
  id: string
  severity: 'warning' | 'critical'
  observed: number
  threshold: number
}

type ProxyOperationalSnapshot = {
  windowSeconds: number
  requestCount: number
  failedRequestCount: number
  errorRatePercent: number | null
  queueDepth: number
  queuedRequestsTotal: number
  queueTimeoutsTotal: number
  queueWaitP95Ms: number | null
  ttftSampleCount: number
  ttftP50Ms: number | null
  ttftP95Ms: number | null
  promptTokensObserved: number
  cachedPromptTokens: number
  cacheReusePercent: number | null
  inFlightRequests: number
  maxConcurrentRequests: number
  saturationPercent: number
  alerts: ProxyOperationalAlert[]
}

type NumericProxyConfigKey = 'connectTimeoutMs' | 'timeoutMs' | 'streamingIdleTimeoutMs'
  | 'healthCheckIntervalMs' | 'healthCheckTimeoutMs' | 'unhealthyThreshold'
  | 'recoveryCooldownMs' | 'maxConcurrentRequests' | 'queueTimeoutMs' | 'requestsPerMinute'
  | 'localityTtlMs' | 'localityMaxEntries'

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

const STORED_API_KEY_PREFIX = 'sha256:'
const SECRET_REVEAL_DURATION_MS = 10_000

function isStoredApiKey(value: string) {
  return value.startsWith(STORED_API_KEY_PREFIX)
}

const defaultConfig: ProxyConfig = {
  enabled: false,
  host: '127.0.0.1',
  port: 11435,
  legacyPublicApiKey: '',
  defaultInstanceId: '',
  routingStrategy: 'priorityFailover',
  strictModelRouting: true,
  localityRoutingEnabled: true,
  localityTtlMs: 1800000,
  localityMaxEntries: 10000,
  connectTimeoutMs: 5000,
  timeoutMs: 600000,
  streamingIdleTimeoutMs: 300000,
  healthCheckIntervalMs: 5000,
  healthCheckTimeoutMs: 2000,
  unhealthyThreshold: 3,
  recoveryCooldownMs: 15000,
  maxConcurrentRequests: 64,
  queueTimeoutMs: 1000,
  requestsPerMinute: 0,
  corsAllowedOrigins: [],
  apiKeys: [],
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

function getOptionalNumber(record: Record<string, unknown>, keys: string[]) {
  for (const key of keys) {
    const value = record[key]
    if (typeof value === 'number' && Number.isFinite(value)) return value
  }
  return null
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
    weight: getNumber(record, ['weight'], 1),
    maxConcurrentRequests: getNumber(record, ['max_concurrent_requests', 'maxConcurrentRequests'], 0),
    modelAlias: getString(record, ['model_alias', 'modelAlias', 'model_pattern', 'modelPattern', 'model']),
    targetInstanceId: getString(record, ['target_instance_id', 'targetInstanceId', 'target_id', 'targetId', 'instance_id', 'instanceId']),
  }
}

function normalizeApiKey(value: unknown, index: number): ProxyApiKey {
  const record = asRecord(value)
  return {
    id: getString(record, ['id'], `key-${index + 1}`),
    name: getString(record, ['name'], `API Key ${index + 1}`),
    key: getString(record, ['key']),
    enabled: getBoolean(record, ['enabled'], true),
    scopes: Array.isArray(record.scopes) ? record.scopes.filter((scope): scope is string => typeof scope === 'string') : ['inference', 'discovery'],
    requestsPerMinute: getNumber(record, ['requests_per_minute', 'requestsPerMinute'], 0),
  }
}

function normalizeConfig(value: unknown): ProxyConfig {
  const record = asRecord(value)
  const routesValue = Array.isArray(record.routes) ? record.routes : []
  const apiKeysValue = Array.isArray(record.api_keys) ? record.api_keys : Array.isArray(record.apiKeys) ? record.apiKeys : []
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
    legacyPublicApiKey: getString(record, ['public_api_key', 'legacyPublicApiKey']),
    defaultInstanceId: getString(record, ['default_instance_id', 'defaultInstanceId', 'default_target_id', 'defaultTargetId']),
    routingStrategy: getString(record, ['routing_strategy', 'routingStrategy'], defaultConfig.routingStrategy),
    strictModelRouting: getBoolean(record, ['strict_model_routing', 'strictModelRouting'], defaultConfig.strictModelRouting),
    localityRoutingEnabled: getBoolean(record, ['locality_routing_enabled', 'localityRoutingEnabled'], defaultConfig.localityRoutingEnabled),
    localityTtlMs: getNumber(record, ['locality_ttl_ms', 'localityTtlMs'], defaultConfig.localityTtlMs),
    localityMaxEntries: getNumber(record, ['locality_max_entries', 'localityMaxEntries'], defaultConfig.localityMaxEntries),
    connectTimeoutMs: getNumber(record, ['connect_timeout_ms', 'connectTimeoutMs'], defaultConfig.connectTimeoutMs),
    timeoutMs: getNumber(record, ['timeout_ms', 'timeoutMs'], defaultConfig.timeoutMs),
    streamingIdleTimeoutMs: getNumber(record, ['streaming_idle_timeout_ms', 'streamingIdleTimeoutMs'], defaultConfig.streamingIdleTimeoutMs),
    healthCheckIntervalMs: getNumber(record, ['health_check_interval_ms', 'healthCheckIntervalMs'], defaultConfig.healthCheckIntervalMs),
    healthCheckTimeoutMs: getNumber(record, ['health_check_timeout_ms', 'healthCheckTimeoutMs'], defaultConfig.healthCheckTimeoutMs),
    unhealthyThreshold: getNumber(record, ['unhealthy_threshold', 'unhealthyThreshold'], defaultConfig.unhealthyThreshold),
    recoveryCooldownMs: getNumber(record, ['recovery_cooldown_ms', 'recoveryCooldownMs'], defaultConfig.recoveryCooldownMs),
    maxConcurrentRequests: getNumber(record, ['max_concurrent_requests', 'maxConcurrentRequests'], defaultConfig.maxConcurrentRequests),
    queueTimeoutMs: getNumber(record, ['queue_timeout_ms', 'queueTimeoutMs'], defaultConfig.queueTimeoutMs),
    requestsPerMinute: getNumber(record, ['requests_per_minute', 'requestsPerMinute'], defaultConfig.requestsPerMinute),
    corsAllowedOrigins: (Array.isArray(record.cors_allowed_origins) ? record.cors_allowed_origins : Array.isArray(record.corsAllowedOrigins) ? record.corsAllowedOrigins : []).filter((origin): origin is string => typeof origin === 'string'),
    apiKeys: apiKeysValue.map(normalizeApiKey),
    backgroundServiceMode: getBoolean(record, ['background_service_mode', 'backgroundServiceMode'], defaultConfig.backgroundServiceMode),
    runtimeServiceEnabled: getBoolean(record, ['runtime_service_enabled', 'runtimeServiceEnabled'], defaultConfig.runtimeServiceEnabled),
    routes,
  }
}

function normalizeOperationalStatus(value: unknown, config: ProxyConfig, inFlightRequests: number): ProxyOperationalSnapshot {
  const record = asRecord(value)
  const alertsValue = Array.isArray(record.alerts) ? record.alerts : []
  const maxConcurrentRequests = Math.max(1, getNumber(record, ['max_concurrent_requests', 'maxConcurrentRequests'], config.maxConcurrentRequests))
  return {
    windowSeconds: getNumber(record, ['window_seconds', 'windowSeconds'], 300),
    requestCount: getNumber(record, ['request_count', 'requestCount']),
    failedRequestCount: getNumber(record, ['failed_request_count', 'failedRequestCount']),
    errorRatePercent: getOptionalNumber(record, ['error_rate_percent', 'errorRatePercent']),
    queueDepth: getNumber(record, ['queue_depth', 'queueDepth']),
    queuedRequestsTotal: getNumber(record, ['queued_requests_total', 'queuedRequestsTotal']),
    queueTimeoutsTotal: getNumber(record, ['queue_timeouts_total', 'queueTimeoutsTotal']),
    queueWaitP95Ms: getOptionalNumber(record, ['queue_wait_p95_ms', 'queueWaitP95Ms']),
    ttftSampleCount: getNumber(record, ['ttft_sample_count', 'ttftSampleCount']),
    ttftP50Ms: getOptionalNumber(record, ['ttft_p50_ms', 'ttftP50Ms']),
    ttftP95Ms: getOptionalNumber(record, ['ttft_p95_ms', 'ttftP95Ms']),
    promptTokensObserved: getNumber(record, ['prompt_tokens_observed', 'promptTokensObserved']),
    cachedPromptTokens: getNumber(record, ['cached_prompt_tokens', 'cachedPromptTokens']),
    cacheReusePercent: getOptionalNumber(record, ['cache_reuse_percent', 'cacheReusePercent']),
    inFlightRequests: getNumber(record, ['in_flight_requests', 'inFlightRequests'], inFlightRequests),
    maxConcurrentRequests,
    saturationPercent: getNumber(record, ['saturation_percent', 'saturationPercent'], inFlightRequests / maxConcurrentRequests * 100),
    alerts: alertsValue.map(value => {
      const alert = asRecord(value)
      return {
        id: getString(alert, ['id'], 'unknown'),
        severity: getString(alert, ['severity']) === 'critical' ? 'critical' as const : 'warning' as const,
        observed: getNumber(alert, ['observed']),
        threshold: getNumber(alert, ['threshold']),
      }
    }),
  }
}

function normalizeStatus(value: unknown, config: ProxyConfig): ProxyStatus {
  const record = asRecord(value)
  const inFlightRequests = getNumber(record, ['in_flight_requests', 'inFlightRequests'], 0)
  return {
    running: getBoolean(record, ['running', 'is_running', 'isRunning'], false),
    boundAddr: getString(record, ['bound_addr', 'boundAddr', 'endpoint', 'url'], formatHostPort(config.host, config.port)),
    activeRoutes: getNumber(record, ['active_routes', 'activeRoutes'], config.routes.filter(route => route.enabled).length),
    healthyRoutes: getNumber(record, ['healthy_routes', 'healthyRoutes'], 0),
    unhealthyRoutes: getNumber(record, ['unhealthy_routes', 'unhealthyRoutes'], 0),
    inFlightRequests,
    totalRequests: getNumber(record, ['total_requests', 'totalRequests'], 0),
    operational: normalizeOperationalStatus(record.operational, config, inFlightRequests),
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
    public_api_key: config.legacyPublicApiKey,
    default_instance_id: config.defaultInstanceId,
    routing_strategy: config.routingStrategy,
    strict_model_routing: config.strictModelRouting,
    locality_routing_enabled: config.localityRoutingEnabled,
    locality_ttl_ms: config.localityTtlMs,
    locality_max_entries: config.localityMaxEntries,
    connect_timeout_ms: config.connectTimeoutMs,
    timeout_ms: config.timeoutMs,
    streaming_idle_timeout_ms: config.streamingIdleTimeoutMs,
    health_check_interval_ms: config.healthCheckIntervalMs,
    health_check_timeout_ms: config.healthCheckTimeoutMs,
    unhealthy_threshold: config.unhealthyThreshold,
    recovery_cooldown_ms: config.recoveryCooldownMs,
    max_concurrent_requests: config.maxConcurrentRequests,
    queue_timeout_ms: config.queueTimeoutMs,
    requests_per_minute: config.requestsPerMinute,
    cors_allowed_origins: config.corsAllowedOrigins,
    api_keys: config.apiKeys.map(apiKey => ({
      id: apiKey.id,
      name: apiKey.name.trim(),
      key: apiKey.key.trim(),
      enabled: apiKey.enabled,
      scopes: apiKey.scopes,
      requests_per_minute: apiKey.requestsPerMinute,
    })),
    background_service_mode: config.backgroundServiceMode,
    runtime_service_enabled: config.runtimeServiceEnabled,
    routes: config.routes.map(route => ({
      id: route.id,
      enabled: route.enabled,
      priority: route.priority,
      weight: route.weight,
      max_concurrent_requests: route.maxConcurrentRequests,
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
    && left.weight === right.weight
    && left.maxConcurrentRequests === right.maxConcurrentRequests
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

function formatOperationalMs(value: number | null) {
  return value == null ? '—' : `${Math.round(value).toLocaleString()} ms`
}

function formatOperationalPercent(value: number | null) {
  return value == null ? '—' : `${value.toFixed(1)}%`
}

function operationalAlertCopy(id: string, labels: ReturnType<typeof getProxyLabels>) {
  if (id === 'error_rate') return { title: labels.alertErrorRate, action: labels.alertErrorRateAction }
  if (id === 'ttft_p95') return { title: labels.alertTtft, action: labels.alertTtftAction }
  if (id === 'queue_wait_p95') return { title: labels.alertQueueWait, action: labels.alertQueueWaitAction }
  if (id === 'queue_timeouts') return { title: labels.alertQueueTimeouts, action: labels.alertQueueTimeoutsAction }
  if (id === 'saturation') return { title: labels.alertSaturation, action: labels.alertSaturationAction }
  return { title: labels.alertUnknown, action: labels.alertUnknownAction }
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
  const [revealedSecretId, setRevealedSecretId] = useState<string | null>(null)
  const revealTimerRef = useRef<number | null>(null)

  const labels = useMemo(() => getProxyLabels(lang), [lang])

  const hideRevealedSecret = () => {
    if (revealTimerRef.current != null) {
      window.clearTimeout(revealTimerRef.current)
      revealTimerRef.current = null
    }
    setRevealedSecretId(null)
  }

  const toggleSecretVisibility = (secretId: string) => {
    if (revealedSecretId === secretId) {
      hideRevealedSecret()
      return
    }
    if (revealTimerRef.current != null) window.clearTimeout(revealTimerRef.current)
    setRevealedSecretId(secretId)
    revealTimerRef.current = window.setTimeout(() => {
      setRevealedSecretId(current => current === secretId ? null : current)
      revealTimerRef.current = null
    }, SECRET_REVEAL_DURATION_MS)
  }

  useEffect(() => () => {
    if (revealTimerRef.current != null) window.clearTimeout(revealTimerRef.current)
  }, [])

  const isLocalHost = (host: string) => {
    const normalized = host.trim().replace(/^\[|\]$/g, '').toLowerCase()
    const octets = normalized.split('.')
    const loopbackIpv4 = octets.length === 4
      && octets[0] === '127'
      && octets.every(octet => /^\d{1,3}$/.test(octet) && Number(octet) <= 255)
    return normalized === '' || normalized === 'localhost' || normalized === '::1' || loopbackIpv4
  }
  const requiresLoopbackHost = !isLocalHost(draft.host)
  const hasApiKeyIssues = draft.apiKeys.some(apiKey => apiKey.enabled && apiKey.key.trim().length < 16)

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
  const apiEndpoints = {
    openAi: `${endpoint}/v1/chat/completions`,
    responses: `${endpoint}/v1/responses`,
    anthropic: `${endpoint}/v1/messages`,
    countTokens: `${endpoint}/v1/messages/count_tokens`,
    models: `${endpoint}/v1/models`,
    slots: `${endpoint}/slots`,
    readiness: `${endpoint}/ready`,
    metrics: `${endpoint}/metrics`,
  }
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
      if (!candidates[0]) continue
      selectedRouteIds.add(candidates[0].route.id)
      if (config.routingStrategy !== 'priorityFailover') {
        const bestPriority = candidates[0].route.priority
        candidates.filter(candidate => candidate.route.priority === bestPriority).forEach(candidate => selectedRouteIds.add(candidate.route.id))
      }
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
  }, [config.routes, config.routingStrategy, draft.routes, effectiveTargets])

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
    let inFlight = false
    const refreshLiveState = async () => {
      if (inFlight) return
      inFlight = true
      const [statusResult, targetsResult, runtimeResult] = await Promise.allSettled([
        invoke<unknown>('get_proxy_status'),
        invoke<unknown[]>('list_proxy_targets'),
        invoke<unknown>('get_runtime_service_status'),
      ])
      if (cancelled) {
        inFlight = false
        return
      }
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
      inFlight = false
    }
    void refreshLiveState()
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

  const updateNumericDraft = (key: NumericProxyConfigKey, value: number) => {
    setDraft(current => ({ ...current, [key]: value }))
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

  const updateApiKey = (id: string, patch: Partial<ProxyApiKey>) => {
    setDraft(current => ({
      ...current,
      apiKeys: current.apiKeys.map(apiKey => apiKey.id === id ? { ...apiKey, ...patch } : apiKey),
    }))
  }

  const addApiKey = () => {
    setDraft(current => ({
      ...current,
      apiKeys: [...current.apiKeys, {
        id: crypto.randomUUID(),
        name: `API Key ${current.apiKeys.length + 1}`,
        key: `lsm_${crypto.randomUUID().replace(/-/g, '')}`,
        enabled: true,
        scopes: ['inference', 'discovery'],
        requestsPerMinute: 0,
      }],
    }))
  }

  const removeApiKey = (id: string) => {
    setDraft(current => ({ ...current, apiKeys: current.apiKeys.filter(apiKey => apiKey.id !== id) }))
  }

  const toggleApiKeyScope = (apiKey: ProxyApiKey, scope: string) => {
    updateApiKey(apiKey.id, {
      scopes: apiKey.scopes.includes(scope)
        ? apiKey.scopes.filter(candidate => candidate !== scope)
        : [...apiKey.scopes, scope],
    })
  }

  const copyApiKey = async (apiKey: ProxyApiKey) => {
    if (isStoredApiKey(apiKey.key)) return
    try {
      await navigator.clipboard.writeText(apiKey.key)
      setNotice(labels.apiKeyCopied)
    } catch {
      // ignore clipboard failures
    }
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
          weight: 1,
          maxConcurrentRequests: 0,
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
    if (hasRouteIssues || hasApiKeyIssues) {
      setError(hasRouteIssues ? labels.routeValidationSummary : labels.apiKeyValidation)
      return
    }
    hideRevealedSecret()
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
    hideRevealedSecret()
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
            <Button data-testid="proxy-header-save" onClick={saveConfig} disabled={saving || hasRouteIssues || hasApiKeyIssues} variant="primary" icon={<Save className="h-4 w-4" />}>
              {saving ? labels.saving : labels.save}
            </Button>
            {statusFresh && status.running ? (
              <Button onClick={() => setProxyRunning('stop')} disabled={busyAction !== null} variant="danger" icon={<Square className="h-4 w-4" />}>
                {labels.stop}
              </Button>
            ) : (
              <Button onClick={() => setProxyRunning('start')} disabled={!statusFresh || busyAction !== null || requiresLoopbackHost || hasRouteIssues || hasApiKeyIssues} variant="success" icon={<Zap className="h-4 w-4" />}>
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
        <MetricCard label={labels.requests} value={statusFresh ? status.totalRequests : '—'} icon={<Route className="h-5 w-5" />} />
        <MetricCard label={labels.inFlightRequests} value={statusFresh ? status.inFlightRequests : '—'} icon={<Zap className="h-5 w-5" />} />
        <MetricCard label={labels.healthyRoutes} value={statusFresh ? `${status.healthyRoutes}/${status.activeRoutes}` : '—'} icon={<HeartPulse className="h-5 w-5" />} />
      </div>

      <Surface as="section" className="p-5" data-testid="proxy-operational-metrics">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.operationalMetrics}</h3>
            <p className="mt-1 text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.operationalMetricsHint}</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Badge tone={statusFresh && status.running ? 'emerald' : 'slate'}>{statusFresh && status.running ? labels.liveWindow : labels.unavailable}</Badge>
            <Badge tone="blue">{Math.round(status.operational.windowSeconds / 60)} min</Badge>
          </div>
        </div>
        <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-6">
          {[
            [labels.ttftP95, formatOperationalMs(status.operational.ttftP95Ms), `${status.operational.ttftSampleCount} ${labels.samples}`],
            [labels.queueDepth, statusFresh ? status.operational.queueDepth.toLocaleString() : '—', `${labels.queueP95}: ${formatOperationalMs(status.operational.queueWaitP95Ms)}`],
            [labels.cacheReuse, formatOperationalPercent(status.operational.cacheReusePercent), `${status.operational.cachedPromptTokens.toLocaleString()} / ${status.operational.promptTokensObserved.toLocaleString()} ${labels.tokens}`],
            [labels.errorRate, formatOperationalPercent(status.operational.errorRatePercent), `${status.operational.failedRequestCount.toLocaleString()} / ${status.operational.requestCount.toLocaleString()}`],
            [labels.saturation, formatOperationalPercent(statusFresh ? status.operational.saturationPercent : null), `${status.operational.inFlightRequests} / ${status.operational.maxConcurrentRequests}`],
            [labels.windowRequests, statusFresh ? status.operational.requestCount.toLocaleString() : '—', `${status.operational.queueTimeoutsTotal.toLocaleString()} ${labels.queueTimeouts}`],
          ].map(([label, value, detail]) => (
            <div key={label} className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 dark:border-slate-800 dark:bg-slate-950/70">
              <div className="text-[11px] font-medium uppercase tracking-wide text-slate-500 dark:text-slate-400">{label}</div>
              <div className="mt-1 text-lg font-semibold text-slate-950 dark:text-slate-50">{value}</div>
              <div className="mt-1 text-xs text-slate-500 dark:text-slate-400">{detail}</div>
            </div>
          ))}
        </div>
        <div className="mt-4 space-y-2" data-operational-alert-count={status.operational.alerts.length}>
          {statusFresh && status.running && status.operational.alerts.length === 0 ? (
            <div className="rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-800 dark:border-emerald-500/30 dark:bg-emerald-500/10 dark:text-emerald-200">
              {labels.noOperationalAlerts}
            </div>
          ) : null}
          {status.operational.alerts.map(alert => {
            const copy = operationalAlertCopy(alert.id, labels)
            const critical = alert.severity === 'critical'
            return (
              <div key={`${alert.id}-${alert.severity}`} className={`rounded-lg border px-3 py-2 text-sm ${critical ? 'border-red-200 bg-red-50 text-red-800 dark:border-red-500/30 dark:bg-red-500/10 dark:text-red-200' : 'border-amber-200 bg-amber-50 text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200'}`}>
                <div className="flex flex-wrap items-center gap-2 font-semibold">
                  <AlertTriangle className="h-4 w-4" />
                  <span>{copy.title}</span>
                  <Badge tone={critical ? 'red' : 'amber'}>{critical ? labels.critical : labels.warning}</Badge>
                </div>
                <p className="mt-1 text-xs leading-5 opacity-90">{copy.action}</p>
              </div>
            )
          })}
        </div>
      </Surface>

      <CanaryRolloutPanel proxyRunning={statusFresh && status.running} targets={effectiveTargets} />

      <div className="grid gap-5 2xl:grid-cols-[minmax(0,1fr)_360px]">
        <div className="min-w-0 space-y-5">
          <Surface as="section" className="p-5">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.apiCompatibility}</h3>
                <p className="mt-1 text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.apiCompatibilityDesc}</p>
              </div>
              <div className="flex gap-2">
                <Badge tone="emerald">OpenAI</Badge>
                <Badge tone="blue">Anthropic</Badge>
              </div>
            </div>
            <div className="mt-4 space-y-2">
              {[
                [labels.openAiEndpoint, apiEndpoints.openAi],
                [labels.responsesEndpoint, apiEndpoints.responses],
                [labels.anthropicEndpoint, apiEndpoints.anthropic],
                [labels.tokenCountEndpoint, apiEndpoints.countTokens],
                [labels.modelDiscoveryEndpoint, apiEndpoints.models],
                [labels.slotsEndpoint, apiEndpoints.slots],
                [labels.readinessEndpoint, apiEndpoints.readiness],
                [labels.metricsEndpoint, apiEndpoints.metrics],
              ].map(([label, value]) => (
                <div key={label} className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 dark:border-slate-800 dark:bg-slate-950/70">
                  <div className="text-[11px] font-medium uppercase text-slate-500 dark:text-slate-400">{label}</div>
                  <div className="mt-1 break-all font-mono text-xs text-slate-800 dark:text-slate-200">{value}</div>
                </div>
              ))}
            </div>
            <div className="mt-4 space-y-2 text-xs leading-5 text-slate-500 dark:text-slate-400">
              <p>{labels.anthropicToolsHint}</p>
              <p>{labels.anthropicScopeHint}</p>
            </div>
          </Surface>

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
            {requiresLoopbackHost ? (
              <div className="mt-3 flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs leading-5 text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <span>{labels.loopbackOnlyHint}</span>
              </div>
            ) : null}

            <div className="mt-3 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
              <label className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.routingStrategy}</span>
                <SelectInput value={draft.routingStrategy} onChange={event => updateDraft({ routingStrategy: event.target.value })} className="w-full">
                  <option value="priorityFailover">{labels.priorityFailover}</option>
                  <option value="roundRobin">{labels.roundRobin}</option>
                  <option value="leastBusy">{labels.leastBusy}</option>
                  <option value="weighted">{labels.weighted}</option>
                </SelectInput>
              </label>
              <div className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.strictRouting}</span>
                <button
                  type="button"
                  role="switch"
                  aria-label={labels.strictRouting}
                  aria-checked={draft.strictModelRouting}
                  onClick={() => updateDraft({ strictModelRouting: !draft.strictModelRouting })}
                  className={`flex h-10 w-full items-center justify-between rounded-lg border px-3 text-sm transition ${draft.strictModelRouting ? 'border-emerald-500/50 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300' : 'border-slate-300 text-slate-600 dark:border-slate-700 dark:text-slate-300'}`}
                >
                  <span>{draft.strictModelRouting ? labels.enabled : labels.disabled}</span>
                  <span className={`relative inline-flex h-6 w-11 rounded-full ${draft.strictModelRouting ? 'bg-emerald-600' : 'bg-slate-300 dark:bg-slate-700'}`}>
                    <span className={`absolute top-1 h-4 w-4 rounded-full bg-white transition ${draft.strictModelRouting ? 'left-6' : 'left-1'}`} />
                  </span>
                </button>
              </div>
            </div>
            <p className="mt-2 text-xs leading-5 text-slate-500 dark:text-slate-400">{labels.strictRoutingHint}</p>

            <div className="mt-4 grid gap-3 md:grid-cols-[220px_minmax(0,1fr)_minmax(0,1fr)]">
              <div className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.localityRouting}</span>
                <button
                  type="button"
                  role="switch"
                  aria-label={labels.localityRouting}
                  aria-checked={draft.localityRoutingEnabled}
                  onClick={() => updateDraft({ localityRoutingEnabled: !draft.localityRoutingEnabled })}
                  className={`flex h-10 w-full items-center justify-between rounded-lg border px-3 text-sm transition ${draft.localityRoutingEnabled ? 'border-emerald-500/50 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300' : 'border-slate-300 text-slate-600 dark:border-slate-700 dark:text-slate-300'}`}
                >
                  <span>{draft.localityRoutingEnabled ? labels.enabled : labels.disabled}</span>
                  <span className={`relative inline-flex h-6 w-11 rounded-full ${draft.localityRoutingEnabled ? 'bg-emerald-600' : 'bg-slate-300 dark:bg-slate-700'}`}>
                    <span className={`absolute top-1 h-4 w-4 rounded-full bg-white transition ${draft.localityRoutingEnabled ? 'left-6' : 'left-1'}`} />
                  </span>
                </button>
              </div>
              <label className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.localityTtl}</span>
                <TextInput
                  type="number"
                  min={60000}
                  max={86400000}
                  value={draft.localityTtlMs}
                  onChange={event => updateNumericDraft('localityTtlMs', Math.max(60000, Math.min(86400000, Number(event.target.value) || 60000)))}
                />
              </label>
              <label className="min-w-0">
                <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.localityCapacity}</span>
                <TextInput
                  type="number"
                  min={1}
                  max={100000}
                  value={draft.localityMaxEntries}
                  onChange={event => updateNumericDraft('localityMaxEntries', Math.max(1, Math.min(100000, Number(event.target.value) || 1)))}
                />
              </label>
            </div>
            <p className="mt-2 text-xs leading-5 text-slate-500 dark:text-slate-400">{labels.localityRoutingHint}</p>

            {status.lastError ? (
              <div className="mt-4 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200">
                <span className="font-semibold">{labels.lastError}: </span>{status.lastError}
              </div>
            ) : null}
          </Surface>

          <Surface as="section" className="p-5">
            <div>
              <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.resilience}</h3>
              <p className="mt-1 text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.resilienceHint}</p>
            </div>
            <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
              {([
                ['timeoutMs', labels.timeout, 1000],
                ['connectTimeoutMs', labels.connectTimeout, 100],
                ['streamingIdleTimeoutMs', labels.streamingIdleTimeout, 1000],
                ['healthCheckIntervalMs', labels.healthCheckInterval, 1000],
                ['healthCheckTimeoutMs', labels.healthCheckTimeout, 250],
                ['unhealthyThreshold', labels.unhealthyThreshold, 1],
                ['recoveryCooldownMs', labels.recoveryCooldown, 1000],
                ['maxConcurrentRequests', labels.maxConcurrentRequests, 1],
                ['queueTimeoutMs', labels.queueTimeout, 10],
                ['requestsPerMinute', labels.requestsPerMinute, 0],
              ] as Array<[NumericProxyConfigKey, string, number]>).map(([key, label, min]) => (
                <label key={key} className="min-w-0">
                  <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{label}</span>
                  <TextInput
                    type="number"
                    min={min}
                    value={draft[key]}
                    onChange={event => updateNumericDraft(key, Math.max(min, Number(event.target.value) || min))}
                  />
                </label>
              ))}
            </div>
          </Surface>

          <Surface as="section" className="p-5">
            <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
              <div>
                <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.accessControl}</h3>
                <p className="mt-1 text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.accessControlHint}</p>
              </div>
              <Button onClick={addApiKey} icon={<Plus className="h-4 w-4" />}>{labels.addApiKey}</Button>
            </div>
            <div className="mt-4 grid gap-3 lg:grid-cols-3">
              {[
                [labels.accessOriginTitle, labels.accessOriginDesc],
                [labels.accessKeyTitle, labels.accessKeyDesc],
                [labels.accessRouteTitle, labels.accessRouteDesc],
              ].map(([title, description], index) => (
                <div key={title} className="flex gap-3 rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-950/70">
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-blue-600 text-xs font-semibold text-white">{index + 1}</span>
                  <div className="min-w-0">
                    <p className="text-sm font-semibold text-slate-900 dark:text-slate-100">{title}</p>
                    <p className="mt-1 text-xs leading-5 text-slate-600 dark:text-slate-400">{description}</p>
                  </div>
                </div>
              ))}
            </div>
            <label className="mt-4 block min-w-0">
              <span className="mb-1 block text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{labels.corsOrigins}</span>
              <TextInput
                value={draft.corsAllowedOrigins.join(', ')}
                placeholder="https://app.example.com, https://admin.example.com"
                onChange={event => updateDraft({ corsAllowedOrigins: event.target.value.split(',').map(origin => origin.trim()).filter(Boolean) })}
              />
              <span className="mt-1.5 block text-xs leading-5 text-slate-500 dark:text-slate-400">{labels.corsOriginsHint}</span>
            </label>
            <p className="mt-4 rounded-lg border border-blue-200 bg-blue-50 px-3 py-2 text-xs leading-5 text-blue-800 dark:border-blue-500/30 dark:bg-blue-500/10 dark:text-blue-200">{labels.apiKeyRelationshipHint}</p>
            <div className="mt-4 space-y-3">
              {draft.apiKeys.length === 0 ? <EmptyPanel title={labels.noApiKeys} /> : draft.apiKeys.map(apiKey => (
                <div key={apiKey.id} className="rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-950/70">
                  <div className="grid gap-3 lg:grid-cols-[180px_minmax(220px,1fr)_160px_auto] lg:items-start">
                    <label className="min-w-0">
                      <span className="mb-1 block text-xs font-medium text-slate-500 dark:text-slate-400">{labels.apiKeyName}</span>
                      <TextInput aria-label={labels.apiKeyName} value={apiKey.name} placeholder={labels.apiKeyName} onChange={event => updateApiKey(apiKey.id, { name: event.target.value })} />
                    </label>
                    <div className="min-w-0">
                      <span className="mb-1 block text-xs font-medium text-slate-500 dark:text-slate-400">{labels.apiKeyValue}</span>
                      <div className="flex min-w-0 items-center gap-2">
                        <TextInput
                          aria-label={labels.apiKeyValue}
                          type={revealedSecretId === `api:${apiKey.id}` && !isStoredApiKey(apiKey.key) ? 'text' : 'password'}
                          autoComplete="off"
                          value={apiKey.key}
                          placeholder={labels.apiKeyValue}
                          onChange={event => updateApiKey(apiKey.id, { key: event.target.value })}
                          className="min-w-0 flex-1"
                        />
                        {apiKey.key && !isStoredApiKey(apiKey.key) ? (
                          <IconButton
                            label={revealedSecretId === `api:${apiKey.id}` ? labels.hideApiKey : labels.revealApiKey}
                            onClick={() => toggleSecretVisibility(`api:${apiKey.id}`)}
                            icon={revealedSecretId === `api:${apiKey.id}` ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                          />
                        ) : null}
                        {apiKey.key && !isStoredApiKey(apiKey.key) ? <IconButton label={labels.copyApiKey} onClick={() => void copyApiKey(apiKey)} icon={<Copy className="h-4 w-4" />} /> : null}
                      </div>
                      {isStoredApiKey(apiKey.key) ? <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{labels.apiKeyHashedHint}</p> : null}
                    </div>
                    <label className="min-w-0">
                      <span className="mb-1 block text-xs font-medium text-slate-500 dark:text-slate-400">{labels.apiKeyRequestsPerMinute}</span>
                      <TextInput type="number" min={0} value={apiKey.requestsPerMinute} onChange={event => updateApiKey(apiKey.id, { requestsPerMinute: Math.max(0, Number(event.target.value) || 0) })} />
                    </label>
                    <div className="flex h-11 items-center justify-end gap-2 lg:mt-5">
                      <button
                        type="button"
                        role="switch"
                        aria-label={labels.enabled}
                        aria-checked={apiKey.enabled}
                        onClick={() => updateApiKey(apiKey.id, { enabled: !apiKey.enabled })}
                        className={`relative inline-flex h-6 w-11 shrink-0 rounded-full transition ${apiKey.enabled ? 'bg-emerald-600' : 'bg-slate-300 dark:bg-slate-700'}`}
                      >
                        <span className={`absolute top-1 h-4 w-4 rounded-full bg-white transition ${apiKey.enabled ? 'left-6' : 'left-1'}`} />
                      </button>
                      <IconButton label={labels.removeApiKey} onClick={() => removeApiKey(apiKey.id)} icon={<Trash2 className="h-4 w-4" />} />
                    </div>
                  </div>
                  <div className="mt-3 flex flex-wrap items-center gap-2">
                    <span className="text-xs text-slate-500 dark:text-slate-400">{labels.scopes}:</span>
                    {['inference', 'discovery'].map(scope => (
                      <button
                        key={scope}
                        type="button"
                        aria-pressed={apiKey.scopes.includes(scope)}
                        onClick={() => toggleApiKeyScope(apiKey, scope)}
                        className={`rounded-full border px-2.5 py-1 text-xs font-medium ${apiKey.scopes.includes(scope) ? 'border-blue-500/40 bg-blue-500/10 text-blue-700 dark:text-blue-300' : 'border-slate-300 text-slate-500 dark:border-slate-700 dark:text-slate-400'}`}
                      >
                        {scope === 'inference' ? labels.inferenceScope : labels.discoveryScope}
                      </button>
                    ))}
                    <span className="ml-auto text-xs text-slate-500 dark:text-slate-400">{labels.apiKeyRpmHint}</span>
                  </div>
                  {apiKey.enabled && apiKey.key.trim().length < 16 ? <p className="mt-2 text-xs text-rose-600 dark:text-rose-400">{labels.apiKeyValidation}</p> : null}
                </div>
              ))}
            </div>
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
                  key: 'weight',
                  header: labels.weight,
                  width: 88,
                  render: route => (
                    <TextInput
                      type="number"
                      min={1}
                      aria-label={labels.weight}
                      value={route.weight}
                      onChange={event => updateRoute(route.id, { weight: Math.max(1, Number(event.target.value) || 1) })}
                      className="h-9"
                    />
                  ),
                },
                {
                  key: 'maxConcurrency',
                  header: labels.routeConcurrency,
                  width: 112,
                  render: route => (
                    <TextInput
                      type="number"
                      min={0}
                      aria-label={labels.routeConcurrency}
                      value={route.maxConcurrentRequests}
                      onChange={event => updateRoute(route.id, { maxConcurrentRequests: Math.max(0, Number(event.target.value) || 0) })}
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
      {(dirty || saving) ? (
        <div
          data-testid="proxy-floating-save"
          role="region"
          aria-label={labels.unsaved}
          aria-live="polite"
          className="fixed bottom-4 right-4 z-30 flex max-w-[calc(100vw-2rem)] items-center gap-3 rounded-xl border border-slate-200 bg-white p-2.5 shadow-xl shadow-slate-950/15 dark:border-slate-700 dark:bg-slate-900 dark:shadow-slate-950/50 sm:bottom-14 sm:right-6"
        >
          <div className="min-w-0 px-1">
            <div className="text-xs font-semibold text-slate-900 dark:text-slate-100">{labels.unsaved}</div>
            {hasRouteIssues || hasApiKeyIssues ? (
              <div className="mt-0.5 max-w-md truncate text-xs text-rose-600 dark:text-rose-300" title={hasRouteIssues ? labels.routeValidationSummary : labels.apiKeyValidation}>
                {hasRouteIssues ? labels.routeValidationSummary : labels.apiKeyValidation}
              </div>
            ) : null}
          </div>
          <Button
            data-testid="proxy-floating-save-button"
            onClick={saveConfig}
            disabled={saving || hasRouteIssues || hasApiKeyIssues}
            variant="primary"
            className="shrink-0"
            icon={<Save className="h-4 w-4" />}
          >
            {saving ? labels.saving : labels.save}
          </Button>
        </div>
      ) : null}
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
