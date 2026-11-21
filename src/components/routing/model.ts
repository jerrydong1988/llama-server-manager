import type { getProxyLabels } from '../../i18n/pageLabels'
import { formatHostPort, httpUrl } from '../../utils/network'

export type ProxyRoute = {
  id: string
  enabled: boolean
  priority: number
  weight: number
  maxConcurrentRequests: number
  modelAlias: string
  targetInstanceId: string
}

export type ProxyApiKey = {
  id: string
  name: string
  key: string
  enabled: boolean
  scopes: string[]
  requestsPerMinute: number
  maxConcurrentRequests: number
  dailyTokenBudget: number
  monthlyTokenBudget: number
  dailyTokenLimit: number
  monthlyTokenLimit: number
  quotaDefaultOutputTokens: number
}

export type ProxyConfig = {
  enabled: boolean
  host: string
  port: number
  legacyPublicApiKey: string
  defaultInstanceId: string
  routingStrategy: string
  strictModelRouting: boolean
  connectTimeoutMs: number
  timeoutMs: number
  streamingIdleTimeoutMs: number
  healthCheckIntervalMs: number
  healthCheckTimeoutMs: number
  unhealthyThreshold: number
  recoveryCooldownMs: number
  maxConcurrentRequests: number
  fairQueueEnabled: boolean
  queueTimeoutMs: number
  requestsPerMinute: number
  corsAllowedOrigins: string[]
  apiKeys: ProxyApiKey[]
  backgroundServiceMode: boolean
  runtimeServiceEnabled: boolean
  routes: ProxyRoute[]
}

export type ProxyStatus = {
  running: boolean
  boundAddr: string
  activeRoutes: number
  healthyRoutes: number
  unhealthyRoutes: number
  inFlightRequests: number
  totalRequests: number
  lastError: string | null
}

export type NumericProxyConfigKey = 'connectTimeoutMs' | 'timeoutMs' | 'streamingIdleTimeoutMs'
  | 'healthCheckIntervalMs' | 'healthCheckTimeoutMs' | 'unhealthyThreshold'
  | 'recoveryCooldownMs' | 'maxConcurrentRequests' | 'queueTimeoutMs' | 'requestsPerMinute'

export type RuntimeServiceView = {
  servicePid: number
  serviceVersion: string
  backgroundEnabled: boolean
  registeredForLogin: boolean
  managedInstances: number
  lastError: string
}

export type ProxyTarget = {
  instanceId: string
  name: string
  alias: string
  endpoint: string
  status: 'running' | 'stopped' | 'unknown'
  source: 'proxy' | 'instances'
}

export type RouteIssue = {
  modelAlias?: string
  targetInstanceId?: string
}

export type RouteAvailabilityKind = 'current' | 'standby' | 'stopped' | 'unknown' | 'missing' | 'disabled' | 'pending' | 'invalid'

export type RouteAvailability = {
  kind: RouteAvailabilityKind
}

export type RouteTestView = {
  tone: 'emerald' | 'amber' | 'red'
  message: string
}

const STORED_API_KEY_PREFIX = 'sha256:'

export function isStoredApiKey(value: string) {
  return value.startsWith(STORED_API_KEY_PREFIX)
}

export const defaultConfig: ProxyConfig = {
  enabled: false,
  host: '127.0.0.1',
  port: 11435,
  legacyPublicApiKey: '',
  defaultInstanceId: '',
  routingStrategy: 'priorityFailover',
  strictModelRouting: true,
  connectTimeoutMs: 5000,
  timeoutMs: 600000,
  streamingIdleTimeoutMs: 300000,
  healthCheckIntervalMs: 5000,
  healthCheckTimeoutMs: 2000,
  unhealthyThreshold: 3,
  recoveryCooldownMs: 15000,
  maxConcurrentRequests: 64,
  fairQueueEnabled: false,
  queueTimeoutMs: 1000,
  requestsPerMinute: 0,
  corsAllowedOrigins: [],
  apiKeys: [],
  backgroundServiceMode: false,
  runtimeServiceEnabled: false,
  routes: [],
}

export const defaultRuntimeService: RuntimeServiceView = {
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
    id: getString(record, ['id']),
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
    id: getString(record, ['id']),
    name: getString(record, ['name'], `API Key ${index + 1}`),
    key: getString(record, ['key']),
    enabled: getBoolean(record, ['enabled'], true),
    scopes: Array.isArray(record.scopes) ? record.scopes.filter((scope): scope is string => typeof scope === 'string') : ['inference', 'discovery'],
    requestsPerMinute: getNumber(record, ['requests_per_minute', 'requestsPerMinute'], 0),
    maxConcurrentRequests: getNumber(record, ['max_concurrent_requests', 'maxConcurrentRequests'], 0),
    dailyTokenBudget: getNumber(record, ['daily_token_budget', 'dailyTokenBudget'], 0),
    monthlyTokenBudget: getNumber(record, ['monthly_token_budget', 'monthlyTokenBudget'], 0),
    dailyTokenLimit: getNumber(record, ['daily_token_limit', 'dailyTokenLimit'], 0),
    monthlyTokenLimit: getNumber(record, ['monthly_token_limit', 'monthlyTokenLimit'], 0),
    quotaDefaultOutputTokens: getNumber(record, ['quota_default_output_tokens', 'quotaDefaultOutputTokens'], 32768) || 32768,
  }
}

function uniqueIds<T extends { id: string }>(items: T[], prefix: string): T[] {
  const reserved = new Set(items.map(item => item.id.trim()))
  const used = new Set<string>()
  return items.map((item, index) => {
    let id = item.id.trim()
    if (!id || used.has(id)) {
      const base = `${prefix}-${index + 1}`
      id = base
      let suffix = 2
      while (used.has(id) || reserved.has(id)) id = `${base}-${suffix++}`
    }
    used.add(id)
    return id === item.id ? item : { ...item, id }
  })
}

export function normalizeConfig(value: unknown): ProxyConfig {
  const record = asRecord(value)
  const routesValue = Array.isArray(record.routes) ? record.routes : []
  const apiKeysValue = Array.isArray(record.api_keys) ? record.api_keys : Array.isArray(record.apiKeys) ? record.apiKeys : []
  return {
    enabled: getBoolean(record, ['enabled'], defaultConfig.enabled),
    host: getString(record, ['host', 'listen_host', 'listenHost'], defaultConfig.host),
    port: getNumber(record, ['port', 'listen_port', 'listenPort'], defaultConfig.port),
    legacyPublicApiKey: getString(record, ['public_api_key', 'legacyPublicApiKey']),
    defaultInstanceId: getString(record, ['default_instance_id', 'defaultInstanceId', 'default_target_id', 'defaultTargetId']),
    routingStrategy: getString(record, ['routing_strategy', 'routingStrategy'], defaultConfig.routingStrategy),
    strictModelRouting: getBoolean(record, ['strict_model_routing', 'strictModelRouting'], defaultConfig.strictModelRouting),
    connectTimeoutMs: getNumber(record, ['connect_timeout_ms', 'connectTimeoutMs'], defaultConfig.connectTimeoutMs),
    timeoutMs: getNumber(record, ['timeout_ms', 'timeoutMs'], defaultConfig.timeoutMs),
    streamingIdleTimeoutMs: getNumber(record, ['streaming_idle_timeout_ms', 'streamingIdleTimeoutMs'], defaultConfig.streamingIdleTimeoutMs),
    healthCheckIntervalMs: getNumber(record, ['health_check_interval_ms', 'healthCheckIntervalMs'], defaultConfig.healthCheckIntervalMs),
    healthCheckTimeoutMs: getNumber(record, ['health_check_timeout_ms', 'healthCheckTimeoutMs'], defaultConfig.healthCheckTimeoutMs),
    unhealthyThreshold: getNumber(record, ['unhealthy_threshold', 'unhealthyThreshold'], defaultConfig.unhealthyThreshold),
    recoveryCooldownMs: getNumber(record, ['recovery_cooldown_ms', 'recoveryCooldownMs'], defaultConfig.recoveryCooldownMs),
    maxConcurrentRequests: getNumber(record, ['max_concurrent_requests', 'maxConcurrentRequests'], defaultConfig.maxConcurrentRequests),
    fairQueueEnabled: getBoolean(record, ['fair_queue_enabled', 'fairQueueEnabled'], false),
    queueTimeoutMs: getNumber(record, ['queue_timeout_ms', 'queueTimeoutMs'], defaultConfig.queueTimeoutMs),
    requestsPerMinute: getNumber(record, ['requests_per_minute', 'requestsPerMinute'], defaultConfig.requestsPerMinute),
    corsAllowedOrigins: (Array.isArray(record.cors_allowed_origins) ? record.cors_allowed_origins : Array.isArray(record.corsAllowedOrigins) ? record.corsAllowedOrigins : []).filter((origin): origin is string => typeof origin === 'string'),
    apiKeys: uniqueIds(apiKeysValue.map(normalizeApiKey), 'key'),
    backgroundServiceMode: getBoolean(record, ['background_service_mode', 'backgroundServiceMode'], defaultConfig.backgroundServiceMode),
    runtimeServiceEnabled: getBoolean(record, ['runtime_service_enabled', 'runtimeServiceEnabled'], defaultConfig.runtimeServiceEnabled),
    routes: uniqueIds(routesValue.map(normalizeRoute), 'route'),
  }
}

export function normalizeStatus(value: unknown, config: ProxyConfig): ProxyStatus {
  const record = asRecord(value)
  return {
    running: getBoolean(record, ['running', 'is_running', 'isRunning'], false),
    boundAddr: getString(record, ['bound_addr', 'boundAddr', 'endpoint', 'url'], formatHostPort(config.host, config.port)),
    activeRoutes: getNumber(record, ['active_routes', 'activeRoutes'], config.routes.filter(route => route.enabled).length),
    healthyRoutes: getNumber(record, ['healthy_routes', 'healthyRoutes'], 0),
    unhealthyRoutes: getNumber(record, ['unhealthy_routes', 'unhealthyRoutes'], 0),
    inFlightRequests: getNumber(record, ['in_flight_requests', 'inFlightRequests'], 0),
    totalRequests: getNumber(record, ['total_requests', 'totalRequests'], 0),
    lastError: getString(record, ['last_error', 'lastError', 'error']) || null,
  }
}

export function normalizeRuntimeService(value: unknown): RuntimeServiceView {
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

export function normalizeTarget(value: unknown, index: number): ProxyTarget {
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

export function toCommandConfig(config: ProxyConfig) {
  return {
    enabled: config.enabled,
    host: config.host,
    port: config.port,
    public_api_key: config.legacyPublicApiKey,
    default_instance_id: config.defaultInstanceId,
    routing_strategy: config.routingStrategy,
    strict_model_routing: config.strictModelRouting,
    connect_timeout_ms: config.connectTimeoutMs,
    timeout_ms: config.timeoutMs,
    streaming_idle_timeout_ms: config.streamingIdleTimeoutMs,
    health_check_interval_ms: config.healthCheckIntervalMs,
    health_check_timeout_ms: config.healthCheckTimeoutMs,
    unhealthy_threshold: config.unhealthyThreshold,
    recovery_cooldown_ms: config.recoveryCooldownMs,
    max_concurrent_requests: config.maxConcurrentRequests,
    fair_queue_enabled: config.fairQueueEnabled,
    queue_timeout_ms: config.queueTimeoutMs,
    requests_per_minute: config.requestsPerMinute,
    cors_allowed_origins: config.corsAllowedOrigins.map(origin => origin.trim()).filter(Boolean),
    api_keys: config.apiKeys.map(apiKey => ({
      id: apiKey.id,
      name: apiKey.name.trim(),
      key: apiKey.key.trim(),
      enabled: apiKey.enabled,
      scopes: apiKey.scopes,
      requests_per_minute: apiKey.requestsPerMinute,
      max_concurrent_requests: apiKey.maxConcurrentRequests,
      daily_token_budget: apiKey.dailyTokenBudget,
      monthly_token_budget: apiKey.monthlyTokenBudget,
      daily_token_limit: apiKey.dailyTokenLimit,
      monthly_token_limit: apiKey.monthlyTokenLimit,
      quota_default_output_tokens: apiKey.quotaDefaultOutputTokens || 32768,
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

export function errorMessage(error: unknown) {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  if (typeof error === 'object' && error !== null && 'message' in error) return String(error.message)
  return String(error)
}

// Apply normalized server values only to fields that still match the submitted snapshot.
export function rebaseDraft(current: ProxyConfig, baseline: ProxyConfig, saved: ProxyConfig): ProxyConfig {
  const merged = { ...saved }
  for (const field of Object.keys(current) as Array<keyof ProxyConfig>) {
    if (current[field] !== baseline[field] && JSON.stringify(current[field]) !== JSON.stringify(baseline[field])) {
      Object.assign(merged, { [field]: current[field] })
    }
  }
  return merged
}

export function endpointUrl(boundAddr: string, config: ProxyConfig) {
  const value = boundAddr || formatHostPort(config.host, config.port)
  return value.startsWith('http://') || value.startsWith('https://') ? value : `http://${value}`
}

export function sameRoute(left: ProxyRoute, right: ProxyRoute) {
  return left.id === right.id
    && left.enabled === right.enabled
    && left.priority === right.priority
    && left.weight === right.weight
    && left.maxConcurrentRequests === right.maxConcurrentRequests
    && left.modelAlias.trim() === right.modelAlias.trim()
    && left.targetInstanceId.trim() === right.targetInstanceId.trim()
}

export function routeAvailabilityView(kind: RouteAvailabilityKind, labels: ReturnType<typeof getProxyLabels>) {
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
