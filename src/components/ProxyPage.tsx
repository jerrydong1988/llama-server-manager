import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Activity, AlertTriangle, Copy, Plus, RefreshCw, Route, Save, Server, Square, Trash2, Zap } from 'lucide-react'
import { useAppStore } from '../store'
import { useI18n } from '../i18n'
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
  routes: ProxyRoute[]
}

type ProxyStatus = {
  running: boolean
  boundAddr: string
  activeRoutes: number
  lastError: string | null
}

type ProxyTarget = {
  instanceId: string
  name: string
  alias: string
  endpoint: string
  status: 'running' | 'stopped' | 'unknown'
  source: 'proxy' | 'instances'
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
  routes: [],
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
  return {
    enabled: getBoolean(record, ['enabled'], defaultConfig.enabled),
    host: getString(record, ['host', 'listen_host', 'listenHost'], defaultConfig.host),
    port: getNumber(record, ['port', 'listen_port', 'listenPort'], defaultConfig.port),
    publicApiKey: getString(record, ['public_api_key', 'publicApiKey']),
    defaultInstanceId: getString(record, ['default_instance_id', 'defaultInstanceId', 'default_target_id', 'defaultTargetId']),
    routingStrategy: getString(record, ['routing_strategy', 'routingStrategy'], defaultConfig.routingStrategy),
    timeoutMs: getNumber(record, ['timeout_ms', 'timeoutMs'], defaultConfig.timeoutMs),
    backgroundServiceMode: getBoolean(record, ['background_service_mode', 'backgroundServiceMode'], defaultConfig.backgroundServiceMode),
    routes: routesValue.map(normalizeRoute),
  }
}

function normalizeStatus(value: unknown, config: ProxyConfig): ProxyStatus {
  const record = asRecord(value)
  return {
    running: getBoolean(record, ['running', 'is_running', 'isRunning'], false),
    boundAddr: getString(record, ['bound_addr', 'boundAddr', 'endpoint', 'url'], `${config.host}:${config.port}`),
    activeRoutes: getNumber(record, ['active_routes', 'activeRoutes'], config.routes.filter(route => route.enabled).length),
    lastError: getString(record, ['last_error', 'lastError', 'error']) || null,
  }
}

function normalizeTarget(value: unknown, index: number): ProxyTarget {
  const record = asRecord(value)
  const host = getString(record, ['host'], '127.0.0.1')
  const port = getNumber(record, ['port'], 0)
  const endpoint = getString(record, ['endpoint', 'url'], port > 0 ? `http://${host}:${port}` : '')
  const rawStatus = getString(record, ['status'], 'unknown').toLowerCase()
  const status: ProxyTarget['status'] = rawStatus === 'running' || rawStatus === 'online'
    ? 'running'
    : rawStatus === 'stopped' || rawStatus === 'offline'
      ? 'stopped'
      : 'unknown'

  return {
    instanceId: getString(record, ['instance_id', 'instanceId', 'id'], `target-${index + 1}`),
    name: getString(record, ['name'], `Target ${index + 1}`),
    alias: getString(record, ['alias']),
    endpoint,
    status: getBoolean(record, ['running'], false) ? 'running' : status,
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
    routes: config.routes.map(route => ({
      id: route.id,
      enabled: route.enabled,
      priority: route.priority,
      model_alias: route.modelAlias,
      target_instance_id: route.targetInstanceId,
    })),
  }
}

function errorMessage(error: unknown) {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return String(error)
}

function endpointUrl(boundAddr: string, config: ProxyConfig) {
  const value = boundAddr || `${config.host}:${config.port}`
  return value.startsWith('http://') || value.startsWith('https://') ? value : `http://${value}`
}

export default function ProxyPage() {
  const { lang } = useI18n()
  const zh = lang === 'zh-CN'
  const instances = useAppStore(state => state.instances)
  const [config, setConfig] = useState<ProxyConfig>(defaultConfig)
  const [draft, setDraft] = useState<ProxyConfig>(defaultConfig)
  const [status, setStatus] = useState<ProxyStatus>(normalizeStatus(null, defaultConfig))
  const [targets, setTargets] = useState<ProxyTarget[]>([])
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [busyAction, setBusyAction] = useState<'start' | 'stop' | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [commandsReady, setCommandsReady] = useState(true)

  const labels = {
    title: zh ? '\u5b9e\u4f8b\u8def\u7531' : 'Instance Routing',
    subtitle: zh ? '\u5bf9\u5916\u63d0\u4f9b\u7edf\u4e00 OpenAI \u517c\u5bb9\u5165\u53e3\uff0c\u6309\u6a21\u578b\u540d\u6216\u522b\u540d\u5206\u53d1\u5230\u540e\u7aef llama-server \u5b9e\u4f8b\u3002' : 'Expose one OpenAI-compatible endpoint and route requests to managed llama-server instances by model name or alias.',
    notReady: zh ? '\u540e\u7aef\u8def\u7531\u547d\u4ee4\u6682\u65f6\u4e0d\u53ef\u7528\uff0c\u8bf7\u5148\u68c0\u67e5\u7f16\u8bd1\u7248\u672c\u3002' : 'Routing commands are unavailable in this build. Check the compiled version first.',
    refresh: zh ? '\u5237\u65b0' : 'Refresh',
    save: zh ? '\u4fdd\u5b58' : 'Save',
    start: zh ? '\u542f\u52a8' : 'Start',
    stop: zh ? '\u505c\u6b62' : 'Stop',
    endpoint: zh ? 'API \u5165\u53e3' : 'API Endpoint',
    status: zh ? '\u72b6\u6001' : 'Status',
    running: zh ? '\u8fd0\u884c\u4e2d' : 'Running',
    stopped: zh ? '\u5df2\u505c\u6b62' : 'Stopped',
    requests: zh ? '\u53ef\u7528\u8def\u7531' : 'Active Routes',
    connections: zh ? '\u5df2\u767b\u8bb0\u76ee\u6807' : 'Targets',
    listen: zh ? '\u76d1\u542c\u5730\u5740' : 'Listen Address',
    host: zh ? '\u4e3b\u673a' : 'Host',
    port: zh ? '\u7aef\u53e3' : 'Port',
    defaultTarget: zh ? '\u9ed8\u8ba4\u5b9e\u4f8b' : 'Default Instance',
    noDefault: zh ? '\u6682\u4e0d\u6307\u5b9a' : 'None',
    routeTable: zh ? '\u8def\u7531\u8868' : 'Route Table',
    addRoute: zh ? '\u6dfb\u52a0\u8def\u7531' : 'Add Route',
    targetList: zh ? '\u76ee\u6807\u5217\u8868' : 'Targets',
    modelPattern: zh ? '\u5bf9\u5916\u6a21\u578b\u540d' : 'Public Model',
    priority: zh ? '\u4f18\u5148\u7ea7' : 'Priority',
    target: zh ? '\u76ee\u6807' : 'Target',
    actions: zh ? '\u64cd\u4f5c' : 'Actions',
    enabled: zh ? '\u542f\u7528' : 'Enabled',
    disabled: zh ? '\u7981\u7528' : 'Disabled',
    copied: zh ? '\u5df2\u590d\u5236\u7aef\u70b9' : 'Endpoint copied',
    saved: zh ? '\u4ee3\u7406\u914d\u7f6e\u5df2\u4fdd\u5b58' : 'Proxy config saved',
    targetFallback: zh ? '\u76ee\u6807\u6765\u81ea\u5f53\u524d\u5b9e\u4f8b\u5217\u8868\uff0c\u53ea\u6709\u8fd0\u884c\u4e2d\u5b9e\u4f8b\u4f1a\u88ab\u4ee3\u7406\u8def\u7531\u547d\u4e2d\u3002' : 'Targets come from the current instance list. Only running instances can receive proxied traffic.',
    noRoutes: zh ? '\u5c1a\u672a\u914d\u7f6e\u8def\u7531\uff0c\u8bf7\u6c42\u5c06\u843d\u5230\u9ed8\u8ba4\u5b9e\u4f8b\u3002' : 'No routes configured. Requests will fall through to the default target.',
    noTargets: zh ? '\u6682\u65e0\u53ef\u7528\u76ee\u6807\u3002' : 'No targets available.',
    publicKey: zh ? '\u4ee3\u7406 API Key\uff08\u53ef\u9009\uff09' : 'Proxy API Key (optional)',
    publicKeyRequired: zh ? '\u76d1\u542c\u975e\u672c\u673a\u5730\u5740\u65f6\u5fc5\u987b\u8bbe\u7f6e\u4ee3\u7406 API Key\u3002' : 'A proxy API key is required when listening on a non-local address.',
    timeout: zh ? '\u8d85\u65f6\uff08\u6beb\u79d2\uff09' : 'Timeout (ms)',
    lastError: zh ? '\u6700\u8fd1\u9519\u8bef' : 'Last Error',
    keepAliveTitle: zh ? '\u540e\u53f0\u4fdd\u6d3b\u6a21\u5f0f' : 'Background keep-alive',
    keepAliveDesc: zh
      ? '\u6258\u76d8\u9000\u51fa\u65f6\u4fdd\u6301\u7ba1\u7406\u5668\u8fdb\u7a0b\u5728\u540e\u53f0\u8fd0\u884c\uff0c\u907f\u514d\u4e2d\u65ad\u5b9e\u4f8b\u8def\u7531\u3002\u5f53\u524d\u9636\u6bb5\u4e0d\u662f\u72ec\u7acb Windows \u670d\u52a1\u3002'
      : 'Keep the manager process alive from the tray so the route is not interrupted. This phase is not a detached Windows service.',
  }

  const isLocalHost = (host: string) => ['', 'localhost', '127.0.0.1', '::1', '[::1]'].includes(host.trim())
  const requiresPublicKey = !isLocalHost(draft.host) && !draft.publicApiKey.trim()

  const fallbackTargets = useMemo<ProxyTarget[]>(() => instances.map(instance => ({
    instanceId: instance.id,
    name: instance.name,
    alias: instance.config.alias,
    endpoint: `http://${instance.config.host}:${instance.config.port}`,
    status: instance.status === 'running' ? 'running' : instance.status === 'stopped' ? 'stopped' : 'unknown',
    source: 'instances',
  })), [instances])

  const effectiveTargets = targets.length > 0 ? targets : fallbackTargets
  const selectedTarget = effectiveTargets.find(target => target.instanceId === draft.defaultInstanceId)
  const endpoint = endpointUrl(status.boundAddr, draft)

  const loadProxy = async () => {
    setLoading(true)
    setError('')
    setNotice('')

    try {
      const configResult = await invoke<unknown>('get_proxy_config')
      const nextConfig = normalizeConfig(configResult)
      setConfig(nextConfig)
      setDraft(nextConfig)
      setCommandsReady(true)

      const [statusResult, targetsResult] = await Promise.allSettled([
        invoke<unknown>('get_proxy_status'),
        invoke<unknown[]>('list_proxy_targets'),
      ])

      if (statusResult.status === 'fulfilled') {
        setStatus(normalizeStatus(statusResult.value, nextConfig))
      } else {
        setStatus(normalizeStatus(null, nextConfig))
      }

      if (targetsResult.status === 'fulfilled' && Array.isArray(targetsResult.value)) {
        setTargets(targetsResult.value.map(normalizeTarget))
      } else {
        setTargets([])
      }
    } catch (loadError) {
      setCommandsReady(false)
      setTargets([])
      setConfig(defaultConfig)
      setDraft(defaultConfig)
      setStatus(normalizeStatus(null, defaultConfig))
      setError(errorMessage(loadError))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void loadProxy()
  }, [])

  const updateDraft = (patch: Partial<ProxyConfig>) => {
    setDraft(current => ({ ...current, ...patch }))
  }

  const updateRoute = (id: string, patch: Partial<ProxyRoute>) => {
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
          priority: current.routes.length + 1,
          modelAlias: '',
          targetInstanceId: current.defaultInstanceId,
        },
      ],
    }))
  }

  const removeRoute = (id: string) => {
    setDraft(current => ({
      ...current,
      routes: current.routes.filter(route => route.id !== id),
    }))
  }

  const saveConfig = async () => {
    setSaving(true)
    setError('')
    setNotice('')

    try {
      await invoke('save_proxy_config', { config: toCommandConfig(draft) })
      setConfig(draft)
      setNotice(labels.saved)
      setCommandsReady(true)
    } catch (saveError) {
      setCommandsReady(false)
      setError(errorMessage(saveError))
    } finally {
      setSaving(false)
    }
  }

  const setProxyRunning = async (action: 'start' | 'stop') => {
    setBusyAction(action)
    setError('')
    setNotice('')

    try {
      await invoke(action === 'start' ? 'start_proxy' : 'stop_proxy')
      const nextStatus = await invoke<unknown>('get_proxy_status').catch(() => null)
      setStatus(normalizeStatus(nextStatus, draft))
      setCommandsReady(true)
    } catch (actionError) {
      setCommandsReady(false)
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

  const dirty = JSON.stringify(config) !== JSON.stringify(draft)

  return (
    <div className="space-y-5">
      <Surface as="section" className="p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
          <div className="min-w-0">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <h2 className="text-2xl font-semibold text-slate-950 dark:text-slate-50">{labels.title}</h2>
              <StatusBadge tone={status.running ? 'emerald' : 'slate'}>
                {status.running ? labels.running : labels.stopped}
              </StatusBadge>
              {!commandsReady ? <Badge tone="amber">{zh ? '\u547d\u4ee4\u4e0d\u53ef\u7528' : 'Unavailable'}</Badge> : null}
              {dirty ? <Badge tone="blue">{zh ? '\u672a\u4fdd\u5b58' : 'Unsaved'}</Badge> : null}
            </div>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.subtitle}</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button onClick={loadProxy} disabled={loading} icon={<RefreshCw className="h-4 w-4" />}>
              {labels.refresh}
            </Button>
            <Button onClick={saveConfig} disabled={saving} variant="primary" icon={<Save className="h-4 w-4" />}>
              {labels.save}
            </Button>
            {status.running ? (
              <Button onClick={() => setProxyRunning('stop')} disabled={busyAction !== null} variant="danger" icon={<Square className="h-4 w-4" />}>
                {labels.stop}
              </Button>
            ) : (
              <Button onClick={() => setProxyRunning('start')} disabled={busyAction !== null || requiresPublicKey} variant="success" icon={<Zap className="h-4 w-4" />}>
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

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <MetricCard label={labels.endpoint} value={endpoint} valueClassName="text-base" icon={<Activity className="h-5 w-5" />} />
        <MetricCard label={labels.defaultTarget} value={selectedTarget?.name || labels.noDefault} valueClassName="text-base" icon={<Server className="h-5 w-5" />} />
        <MetricCard label={labels.connections} value={effectiveTargets.length} icon={<Zap className="h-5 w-5" />} />
        <MetricCard label={labels.requests} value={status.activeRoutes} icon={<Route className="h-5 w-5" />} />
      </div>

      <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_360px]">
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
                    <option key={target.instanceId} value={target.instanceId}>{target.name}</option>
                  ))}
                </SelectInput>
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
                <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">{labels.noRoutes}</p>
              </div>
              <Button onClick={addRoute} variant="primary" icon={<Plus className="h-4 w-4" />}>
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
                  width: 116,
                  render: route => (
                    <button
                      type="button"
                      onClick={() => updateRoute(route.id, { enabled: !route.enabled })}
                      className="inline-flex min-w-[88px] items-center justify-center rounded-md border border-slate-200 px-2.5 py-1 text-xs font-medium text-slate-700 transition hover:bg-slate-50 dark:border-slate-700 dark:text-slate-200 dark:hover:bg-slate-800"
                    >
                      {route.enabled ? labels.enabled : labels.disabled}
                    </button>
                  ),
                },
                {
                  key: 'priority',
                  header: labels.priority,
                  width: 96,
                  render: route => (
                    <TextInput
                      type="number"
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
                  render: route => (
                    <TextInput value={route.modelAlias} onChange={event => updateRoute(route.id, { modelAlias: event.target.value })} className="h-9" />
                  ),
                },
                {
                  key: 'target',
                  header: labels.target,
                  minWidth: 220,
                  render: route => (
                    <SelectInput value={route.targetInstanceId} onChange={event => updateRoute(route.id, { targetInstanceId: event.target.value })} className="h-9 w-full">
                      <option value="">{labels.noDefault}</option>
                      {effectiveTargets.map(target => (
                        <option key={target.instanceId} value={target.instanceId}>{target.name}</option>
                      ))}
                    </SelectInput>
                  ),
                },
                {
                  key: 'actions',
                  header: labels.actions,
                  width: 72,
                  align: 'right',
                  render: route => (
                    <IconButton
                      label={labels.actions}
                      onClick={() => removeRoute(route.id)}
                      icon={<Trash2 className="h-4 w-4" />}
                    />
                  ),
                },
              ]}
            />
          </Surface>
        </div>

        <div className="min-w-0 space-y-5">
          <Surface as="section" className="p-5">
            <div className="mb-4 flex items-start justify-between gap-3">
              <div>
                <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.targetList}</h3>
                <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">
                  {targets.length > 0 ? `${targets.length}` : labels.targetFallback}
                </p>
              </div>
              {targets.length === 0 ? <AlertTriangle className="h-5 w-5 shrink-0 text-amber-500" /> : null}
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
                      {target.status === 'running' ? labels.running : target.status === 'stopped' ? labels.stopped : zh ? '\u672a\u77e5' : 'Unknown'}
                    </StatusBadge>
                  </div>
                  {target.alias ? <div className="mt-2 truncate text-xs text-slate-500 dark:text-slate-400" title={target.alias}>{zh ? '\u522b\u540d' : 'Alias'}: {target.alias}</div> : null}
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
                onClick={() => updateDraft({ backgroundServiceMode: !draft.backgroundServiceMode })}
                className={`relative mt-1 inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition ${
                  draft.backgroundServiceMode
                    ? 'border-blue-500 bg-blue-600'
                    : 'border-slate-300 bg-slate-200 dark:border-slate-700 dark:bg-slate-800'
                }`}
                aria-pressed={draft.backgroundServiceMode}
              >
                <span
                  className={`inline-block h-5 w-5 rounded-full bg-white shadow transition ${
                    draft.backgroundServiceMode ? 'translate-x-6' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>
            <div className="mt-4 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-xs font-medium text-slate-600 dark:border-slate-800 dark:bg-slate-950/70 dark:text-slate-300">
              {draft.backgroundServiceMode ? labels.enabled : labels.disabled}
            </div>
          </Surface>
        </div>
      </div>
    </div>
  )
}
