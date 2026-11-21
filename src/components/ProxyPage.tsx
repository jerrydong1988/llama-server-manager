import { useEffect, useMemo, useRef, useState } from 'react'
import { Activity, Copy, GitFork, KeyRound, RefreshCw, Save, SlidersHorizontal, Square, Zap } from 'lucide-react'
import { listen } from '@tauri-apps/api/event'
import { invokeApp as invoke } from '../lib/ipc'
import { useAppStore } from '../store'
import { httpUrl } from '../utils/network'
import { useI18n } from '../i18n'
import { getProxyLabels } from '../i18n/pageLabels'
import { getRouterUsageLabels } from '../i18n/routerUsage'
import { getRoutingWorkspaceLabels } from '../i18n/routingWorkspace'
import { getAgentManagementLabels } from '../i18n/agentManagement'
import ProxyUsagePanel from './ProxyUsagePanel'
import ApiKeysWorkspace from './routing/ApiKeysWorkspace'
import RouterSettings from './routing/RouterSettings'
import RoutesWorkspace from './routing/RoutesWorkspace'
import {
  defaultConfig,
  defaultRuntimeService,
  endpointUrl,
  errorMessage,
  normalizeConfig,
  normalizeRuntimeService,
  normalizeStatus,
  normalizeTarget,
  sameRoute,
  toCommandConfig,
  rebaseDraft,
  type ProxyApiKey,
  type ProxyConfig,
  type ProxyRoute,
  type ProxyStatus,
  type ProxyTarget,
  type RouteAvailability,
  type RouteIssue,
  type RouteTestView,
  type RuntimeServiceView,
} from './routing/model'
import { WorkspaceFloatingActions } from './shell/WorkspaceActions'
import { Badge, Button, IconButton, StatusBadge } from './ui'
import './routing/workspace.css'

type RoutingView = 'routes' | 'keys' | 'usage' | 'settings'

export default function ProxyPage() {
  const configRevision = useRef<string | undefined>(undefined)
  const { lang } = useI18n()
  const w = getRoutingWorkspaceLabels(lang)
  const ioBusy = useRef(false)
  const [view, setView] = useState<RoutingView>('routes')
  const usageLabels = useMemo(() => getRouterUsageLabels(lang), [lang])
  const instances = useAppStore((state) => state.instances)
  const [config, setConfig] = useState<ProxyConfig>(defaultConfig)
  const [draft, setDraft] = useState<ProxyConfig>(defaultConfig)
  const [status, setStatus] = useState<ProxyStatus>(normalizeStatus(null, defaultConfig))
  const [statusFresh, setStatusFresh] = useState(false)
  const [runtimeService, setRuntimeService] = useState<RuntimeServiceView>(defaultRuntimeService)
  const [runtimeFresh, setRuntimeFresh] = useState(false)
  const [targets, setTargets] = useState<ProxyTarget[]>([])
  const [targetsFresh, setTargetsFresh] = useState(false)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [busyAction, setBusyAction] = useState<'start' | 'stop' | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  useEffect(() => {
    if (!notice) return
    const timeout = window.setTimeout(() => setNotice(''), 5000)
    return () => window.clearTimeout(timeout)
  }, [notice])
  const [commandsReady, setCommandsReady] = useState(true)
  const [testingRouteId, setTestingRouteId] = useState<string | null>(null)
  const [routeTests, setRouteTests] = useState<Record<string, RouteTestView>>({})
  const [stopRuntimeConfirmOpen, setStopRuntimeConfirmOpen] = useState(false)
  const [stoppingRuntime, setStoppingRuntime] = useState(false)
  const [confirmReload, setConfirmReload] = useState(false)
  const labels = useMemo(() => getProxyLabels(lang), [lang])

  const isLocalHost = (host: string) => {
    const normalized = host
      .trim()
      .replace(/^\[|\]$/g, '')
      .toLowerCase()
    const octets = normalized.split('.')
    const loopbackIpv4 =
      octets.length === 4 &&
      octets[0] === '127' &&
      octets.every((octet) => /^\d{1,3}$/.test(octet) && Number(octet) <= 255)
    return normalized === '' || normalized === 'localhost' || normalized === '::1' || loopbackIpv4
  }
  const requiresLoopbackHost = !isLocalHost(draft.host)
  const hasApiKeyIssues = draft.apiKeys.some((apiKey) => apiKey.enabled && apiKey.key.trim().length < 16)

  const fallbackTargets = useMemo<ProxyTarget[]>(
    () =>
      instances.map((instance) => ({
        instanceId: instance.id,
        name: instance.name,
        alias: instance.config.alias,
        endpoint: httpUrl(instance.config.host, instance.config.port),
        status: instance.status === 'running' ? 'running' : instance.status === 'stopped' ? 'stopped' : 'unknown',
        source: 'instances',
      })),
    [instances],
  )

  const displayedTargets = targetsFresh ? targets : targets.length > 0 ? targets : fallbackTargets
  const effectiveTargets = targetsFresh
    ? displayedTargets
    : displayedTargets.map((target) => ({ ...target, status: 'unknown' as const }))
  const selectedTarget = effectiveTargets.find((target) => target.instanceId === draft.defaultInstanceId)
  const endpoint = endpointUrl(status.boundAddr, draft)
  const routeIssues = useMemo(() => {
    const knownTargetIds = new Set(effectiveTargets.map((target) => target.instanceId))
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
    const targetById = new Map(effectiveTargets.map((target) => [target.instanceId, target]))
    const savedById = new Map(config.routes.map((route) => [route.id, route]))
    const candidatesByModel = new Map<string, Array<{ route: ProxyRoute; index: number }>>()

    config.routes.forEach((route, index) => {
      if (!route.enabled || !route.modelAlias.trim() || !route.targetInstanceId.trim()) return
      const target = targetById.get(route.targetInstanceId.trim())
      if (target?.status !== 'running') return
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
        candidates
          .filter((candidate) => candidate.route.priority === bestPriority)
          .forEach((candidate) => selectedRouteIds.add(candidate.route.id))
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

    return byId
  }, [config.routes, config.routingStrategy, draft.routes, effectiveTargets])

  const loadProxy = async () => {
    if (ioBusy.current) return
    ioBusy.current = true
    setLoading(true)
    setError('')
    setNotice('')
    setRouteTests({})

    try {
      const configResult = await invoke<unknown>('get_proxy_config')
      configRevision.current = (configResult as { config_revision?: string } | null)?.config_revision
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
      setStatus(normalizeStatus(null, defaultConfig))
      setStatusFresh(false)
      setRuntimeFresh(false)
      setError(errorMessage(loadError))
    } finally {
      ioBusy.current = false
      setLoading(false)
    }
  }

  useEffect(() => {
    void loadProxy()
  }, [])

  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen<{ operation: string }>('management-config-changed', (event) => {
      if (!event.payload.operation.startsWith('routes.')) return
      if (ioBusy.current || JSON.stringify(config) !== JSON.stringify(draft)) {
        setError(getAgentManagementLabels(lang).routerConflict)
      } else {
        void loadProxy()
      }
    })
      .then((stop) => {
        if (disposed) stop()
        else unlisten = stop
      })
      .catch((error) => setError(errorMessage(error)))
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [config, draft, lang])

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
    setDraft((current) => ({ ...current, ...patch }))
  }

  const updateRoute = (id: string, patch: Partial<ProxyRoute>) => {
    setRouteTests((current) => {
      const next = { ...current }
      delete next[id]
      return next
    })
    setDraft((current) => ({
      ...current,
      routes: current.routes.map((route) => (route.id === id ? { ...route, ...patch } : route)),
    }))
  }

  const addRoute = () => {
    const id = crypto.randomUUID()
    setDraft((current) => ({
      ...current,
      routes: [
        ...current.routes,
        {
          id,
          enabled: true,
          priority: Math.max(0, ...current.routes.map((route) => route.priority)) + 1,
          weight: 1,
          maxConcurrentRequests: 0,
          modelAlias: '',
          targetInstanceId:
            current.defaultInstanceId || (effectiveTargets.length === 1 ? effectiveTargets[0].instanceId : ''),
        },
      ],
    }))
    return id
  }

  const removeRoute = (id: string) => {
    setRouteTests((current) => {
      const next = { ...current }
      delete next[id]
      return next
    })
    setDraft((current) => ({
      ...current,
      routes: current.routes.filter((route) => route.id !== id),
    }))
  }

  const testRoute = async (route: ProxyRoute) => {
    const saved = config.routes.find((candidate) => candidate.id === route.id)
    if (!saved || !sameRoute(route, saved)) {
      setRouteTests((current) => ({
        ...current,
        [route.id]: { tone: 'amber', message: labels.routeTestSaveFirst },
      }))
      return
    }

    setTestingRouteId(route.id)
    setRouteTests((current) => {
      const next = { ...current }
      delete next[route.id]
      return next
    })
    try {
      const resolvedValue = await invoke<unknown>('test_proxy_route', { model: saved.modelAlias.trim() })
      const resolved = normalizeTarget(resolvedValue, 0)
      const selected = resolved.instanceId === saved.targetInstanceId.trim()
      setRouteTests((current) => ({
        ...current,
        [route.id]: {
          tone: selected ? 'emerald' : 'amber',
          message: `${selected ? labels.routeTestHit : labels.routeTestRerouted}: ${resolved.name}`,
        },
      }))
    } catch (testError) {
      setRouteTests((current) => ({
        ...current,
        [route.id]: { tone: 'red', message: `${labels.routeTestFailed}: ${errorMessage(testError)}` },
      }))
    } finally {
      setTestingRouteId(null)
    }
  }

  const dirty = JSON.stringify(config) !== JSON.stringify(draft)
  const busy = loading || saving || busyAction !== null || stoppingRuntime

  async function persist(submitted: ProxyConfig, baseline: ProxyConfig) {
    const response = await invoke<unknown>('save_proxy_config', {
      config: toCommandConfig(submitted),
      expectedRevision: configRevision.current,
    })
    const receipt = response ?? (await invoke<unknown>('get_proxy_config'))
    const saved = normalizeConfig(receipt)
    configRevision.current = (receipt as { config_revision?: string }).config_revision
    setConfig(saved)
    setDraft((current) => rebaseDraft(current, baseline, saved))
    setCommandsReady(true)
    return saved
  }

  async function saveConfig() {
    if (ioBusy.current || !commandsReady || hasRouteIssues || hasApiKeyIssues) return
    ioBusy.current = true
    setSaving(true)
    setError('')
    setNotice('')
    try {
      const saved = await persist(draft, draft)
      const live = await invoke<unknown>('get_proxy_status').catch(() => null)
      if (live) setStatus(normalizeStatus(live, saved))
      setStatusFresh(live != null)
      setRouteTests({})
      setNotice(labels.saved)
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      ioBusy.current = false
      setSaving(false)
    }
  }

  async function saveKey(original: ProxyApiKey | null, next: ProxyApiKey | null) {
    if (ioBusy.current || !commandsReady) return false
    const id = original?.id ?? next?.id
    const existing = config.apiKeys.find((key) => key.id === id)
    if (JSON.stringify(existing ?? null) !== JSON.stringify(original)) {
      setError(w.keyConflict)
      return false
    }
    const apiKeys =
      next === null
        ? config.apiKeys.filter((k) => k.id !== id)
        : original === null
          ? [next, ...config.apiKeys]
          : config.apiKeys.map((k) => (k.id === id ? next : k))
    ioBusy.current = true
    setSaving(true)
    setError('')
    setNotice('')
    try {
      // The command replaces the whole config. Start with the last saved revision,
      // never the unsubmitted routes/settings draft.
      await persist({ ...config, apiKeys }, config)
      setNotice(next ? w.savedKey : w.removedKey)
      return true
    } catch (e) {
      setError(errorMessage(e))
      return false
    } finally {
      ioBusy.current = false
      setSaving(false)
    }
  }
  const setProxyRunning = async (action: 'start' | 'stop') => {
    if (ioBusy.current || !commandsReady) return
    ioBusy.current = true
    setBusyAction(action)
    setError('')
    setNotice('')

    try {
      let effectiveConfig = config
      if (action === 'start' && dirty) {
        effectiveConfig = await persist(draft, draft)
      }
      const actionStatus = await invoke<unknown>(action === 'start' ? 'start_proxy' : 'stop_proxy')
      const nextStatus = await invoke<unknown>('get_proxy_status').catch(() => actionStatus)
      setStatus(normalizeStatus(nextStatus, effectiveConfig))
      setStatusFresh(nextStatus != null)
      const enabled = action === 'start'
      const receipt = await invoke<unknown>('get_proxy_config')
      const savedConfig = normalizeConfig(receipt)
      if (JSON.stringify(savedConfig) !== JSON.stringify({ ...effectiveConfig, enabled })) {
        throw new Error(getAgentManagementLabels(lang).routerConflict)
      }
      configRevision.current = (receipt as { config_revision?: string } | null)?.config_revision
      setConfig(savedConfig)
      setDraft((current) => ({ ...current, enabled }))
      setCommandsReady(true)
    } catch (actionError) {
      setCommandsReady(true)
      setError(errorMessage(actionError))
    } finally {
      ioBusy.current = false
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
    if (ioBusy.current) return
    ioBusy.current = true
    setStoppingRuntime(true)
    setError('')
    setNotice('')
    try {
      await invoke('stop_background_runtime')
      setStopRuntimeConfirmOpen(false)
      ioBusy.current = false
      await loadProxy()
      setNotice(labels.backgroundStopped)
    } catch (stopError) {
      const message = errorMessage(stopError)
      setStopRuntimeConfirmOpen(false)
      ioBusy.current = false
      await loadProxy()
      setError(message)
    } finally {
      setStoppingRuntime(false)
    }
  }

  const tabs = [
    { id: 'routes', label: w.routes, icon: GitFork },
    { id: 'keys', label: 'API Key', icon: KeyRound },
    { id: 'usage', label: usageLabels.title, icon: Activity },
    { id: 'settings', label: w.settings, icon: SlidersHorizontal },
  ] as const
  const configView = view === 'routes' || view === 'settings'
  const saveDisabled = busy || !commandsReady || hasRouteIssues || hasApiKeyIssues
  return (
    <div className="router-workspace space-y-4">
      <section
        aria-label={w.service}
        data-guide="proxy-overview"
        className="flex flex-wrap items-center justify-between gap-2"
      >
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-base font-semibold">{w.service}</h2>
          <StatusBadge tone={!statusFresh ? 'amber' : status.running ? 'emerald' : 'slate'}>
            {!statusFresh ? labels.unknown : status.running ? labels.running : labels.stopped}
          </StatusBadge>
          {!commandsReady && <Badge tone="amber">{labels.unavailable}</Badge>}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="subtle"
            title={labels.endpoint}
            aria-label={`${labels.endpoint} · ${endpoint}`}
            icon={<Copy size={14} />}
            onClick={copyEndpoint}
          >
            <span className="font-mono">{endpoint}</span>
          </Button>
          <IconButton
            className="h-8 w-8"
            label={labels.refresh}
            disabled={busy}
            onClick={() => (dirty ? setConfirmReload(true) : void loadProxy())}
            icon={<RefreshCw size={14} />}
          />
          {configView && dirty && (
            <Button
              size="sm"
              data-testid="proxy-header-save"
              disabled={saveDisabled}
              onClick={() => void saveConfig()}
              icon={<Save size={14} />}
            >
              {labels.save}
            </Button>
          )}
          {statusFresh && status.running ? (
            <Button size="sm" disabled={busy} icon={<Square size={13} />} onClick={() => void setProxyRunning('stop')}>
              {labels.stop}
            </Button>
          ) : (
            <Button
              size="sm"
              variant="success"
              disabled={
                busy || !commandsReady || !statusFresh || requiresLoopbackHost || hasRouteIssues || hasApiKeyIssues
              }
              icon={<Zap size={14} />}
              onClick={() => void setProxyRunning('start')}
            >
              {labels.start}
            </Button>
          )}
        </div>
      </section>
      <div className="router-tabs" role="tablist" aria-label={labels.title}>
        {tabs.map(({ id, label, icon: Icon }, index) => (
          <button
            type="button"
            key={id}
            id={`router-tab-${id}`}
            role="tab"
            aria-selected={view === id}
            aria-controls={`router-panel-${id}`}
            tabIndex={view === id ? 0 : -1}
            onClick={() => setView(id)}
            onKeyDown={(e) => {
              const position =
                e.key === 'ArrowRight'
                  ? index + 1
                  : e.key === 'ArrowLeft'
                    ? index - 1
                    : e.key === 'Home'
                      ? 0
                      : e.key === 'End'
                        ? tabs.length - 1
                        : null
              if (position === null) return
              e.preventDefault()
              const target = tabs[(position + tabs.length) % tabs.length].id
              setView(target)
              document.getElementById(`router-tab-${target}`)?.focus()
            }}
          >
            <Icon size={16} />
            {label}
          </button>
        ))}
      </div>
      {(!commandsReady || error || notice || status.lastError || confirmReload) && (
        <div className="space-y-2 text-sm">
          {!commandsReady && (
            <p role="alert" className="text-amber-700 dark:text-amber-300">
              {labels.notReady}
            </p>
          )}
          {error && (
            <p role="alert" className="text-rose-700 dark:text-rose-300">
              {error}
            </p>
          )}
          {status.lastError && (
            <p role="alert" className="text-rose-700 dark:text-rose-300">
              {labels.lastError} · {status.lastError}
            </p>
          )}
          {notice && (
            <p role="status" className="text-emerald-700 dark:text-emerald-300">
              {notice}
            </p>
          )}
          {confirmReload && (
            <div className="flex flex-wrap items-center gap-2" role="alert">
              <span>{w.reloadPrompt}</span>
              <Button size="sm" onClick={() => setConfirmReload(false)}>
                {labels.cancel}
              </Button>
              <Button
                size="sm"
                disabled={busy}
                onClick={() => {
                  setConfirmReload(false)
                  void loadProxy()
                }}
              >
                {labels.refresh}
              </Button>
            </div>
          )}
        </div>
      )}
      <div id="router-panel-routes" role="tabpanel" aria-labelledby="router-tab-routes" hidden={view !== 'routes'}>
        <RoutesWorkspace
          routes={draft.routes}
          targets={effectiveTargets}
          targetsFresh={targetsFresh}
          availability={routeAvailability}
          metrics={[
            [labels.healthyRoutes, statusFresh ? `${status.healthyRoutes}/${status.activeRoutes}` : '—'],
            [labels.inFlightRequests, statusFresh ? status.inFlightRequests : '—'],
            [labels.requests, statusFresh ? status.totalRequests : '—'],
            [labels.defaultTarget, selectedTarget?.name || labels.noDefault],
          ]}
          issues={routeIssues}
          tests={routeTests}
          testingId={testingRouteId}
          onAdd={addRoute}
          onUpdate={updateRoute}
          onRemove={removeRoute}
          onTest={testRoute}
        />
      </div>
      <div id="router-panel-keys" role="tabpanel" aria-labelledby="router-tab-keys" hidden={view !== 'keys'}>
        <ApiKeysWorkspace
          active={view === 'keys'}
          available={!loading && commandsReady}
          keys={config.apiKeys}
          legacy={Boolean(config.legacyPublicApiKey)}
          busy={busy || !commandsReady}
          onCommit={saveKey}
        />
      </div>
      <div id="router-panel-usage" role="tabpanel" aria-labelledby="router-tab-usage" hidden={view !== 'usage'}>
        {view === 'usage' && <ProxyUsagePanel />}
      </div>
      <div
        id="router-panel-settings"
        role="tabpanel"
        aria-labelledby="router-tab-settings"
        hidden={view !== 'settings'}
      >
        <RouterSettings
          config={config}
          draft={draft}
          targets={effectiveTargets}
          endpoint={endpoint}
          requiresLoopback={requiresLoopbackHost}
          runtime={runtimeService}
          runtimeFresh={runtimeFresh}
          running={status.running}
          busy={busy}
          onChange={updateDraft}
          onStopRuntime={() => setStopRuntimeConfirmOpen(true)}
        />
      </div>
      {configView && dirty && (
        <WorkspaceFloatingActions>
          <div
            data-testid="proxy-floating-save"
            role="region"
            aria-label={labels.unsaved}
            className="pointer-events-auto flex items-center gap-3 rounded-xl border border-slate-200 bg-white p-2.5 shadow-lg dark:border-slate-700 dark:bg-slate-900"
          >
            <div className="min-w-0">
              <p className="text-xs font-semibold">{labels.unsaved}</p>
              {hasRouteIssues && (
                <p className="max-w-xs text-xs text-rose-700 dark:text-rose-300">{labels.routeValidationSummary}</p>
              )}
            </div>
            <Button
              data-testid="proxy-floating-save-button"
              size="sm"
              onClick={() => void saveConfig()}
              disabled={saveDisabled}
              variant="primary"
              icon={<Save size={15} />}
            >
              {saving ? labels.saving : labels.save}
            </Button>
          </div>
        </WorkspaceFloatingActions>
      )}
      {stopRuntimeConfirmOpen ? (
        <div
          className="fixed inset-0 z-[120] flex items-center justify-center bg-slate-950/60 px-4 backdrop-blur-sm"
          role="presentation"
        >
          <div
            className="w-full max-w-lg rounded-xl border border-slate-200 bg-white p-6 shadow-2xl dark:border-slate-800 dark:bg-slate-950"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="stop-background-runtime-title"
          >
            <h2 id="stop-background-runtime-title" className="text-xl font-semibold text-slate-950 dark:text-slate-50">
              {labels.stopRuntimeConfirmTitle}
            </h2>
            <p className="mt-3 text-sm leading-6 text-slate-600 dark:text-slate-300">{labels.stopRuntimeConfirmDesc}</p>
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
