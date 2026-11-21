import type { ReactNode } from 'react'
import { Activity, ChevronRight, GitFork, Globe, Plug, PowerOff, Server } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getProxyLabels } from '../../i18n/pageLabels'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { getRoutingWorkspaceLabels } from '../../i18n/routingWorkspace'
import { Button, SelectInput, TextInput } from '../ui'
import type { NumericProxyConfigKey, ProxyConfig, ProxyTarget, RuntimeServiceView } from './model'

function Group({
  title,
  summary,
  icon,
  children,
}: {
  title: string
  summary: string
  icon: ReactNode
  children: ReactNode
}) {
  return (
    <details className="router-setting">
      <summary>
        {icon}
        <span className="flex-1">{title}</span>
        <span className="router-setting-summary router-muted truncate font-normal">{summary}</span>
        <ChevronRight className="router-chevron" size={16} />
      </summary>
      <div className="router-setting-body">{children}</div>
    </details>
  )
}

type Props = {
  config: ProxyConfig
  draft: ProxyConfig
  targets: ProxyTarget[]
  endpoint: string
  requiresLoopback: boolean
  runtime: RuntimeServiceView
  runtimeFresh: boolean
  running: boolean
  busy: boolean
  onChange: (patch: Partial<ProxyConfig>) => void
  onStopRuntime: () => void
}
export default function RouterSettings({
  config,
  draft,
  targets,
  endpoint,
  requiresLoopback,
  runtime,
  runtimeFresh,
  running,
  busy,
  onChange,
  onStopRuntime,
}: Props) {
  const { lang } = useI18n()
  const l = getProxyLabels(lang),
    w = getRoutingWorkspaceLabels(lang),
    m = getRouterManagementLabels(lang)
  const originCount = draft.corsAllowedOrigins.filter(origin => origin.trim()).length
  const strategies = {
    priorityFailover: l.priorityFailover,
    roundRobin: l.roundRobin,
    leastBusy: l.leastBusy,
    weighted: l.weighted,
  }
  const fields: Array<[NumericProxyConfigKey, string, number]> = [
    ['maxConcurrentRequests', l.maxConcurrentRequests, 1],
    ['requestsPerMinute', l.requestsPerMinute, 0],
    ['queueTimeoutMs', l.queueTimeout, 10],
    ['connectTimeoutMs', l.connectTimeout, 100],
    ['timeoutMs', l.timeout, 1000],
    ['streamingIdleTimeoutMs', l.streamingIdleTimeout, 1000],
    ['healthCheckIntervalMs', l.healthCheckInterval, 1000],
    ['healthCheckTimeoutMs', l.healthCheckTimeout, 250],
    ['unhealthyThreshold', l.unhealthyThreshold, 1],
    ['recoveryCooldownMs', l.recoveryCooldown, 1000],
  ]
  return (
    <section aria-labelledby="router-settings-title">
      <header className="router-section-header">
        <h2 id="router-settings-title">{w.settings}</h2>
      </header>
      <div className="router-panel">
        <Group title={w.listen} summary={endpoint} icon={<Plug size={16} />}>
          <div className="grid max-w-xl grid-cols-[minmax(0,1fr)_7rem] gap-3">
            <label>
              <span className="router-label">{l.host}</span>
              <TextInput value={draft.host} onChange={(e) => onChange({ host: e.target.value })} />
            </label>
            <label>
              <span className="router-label">{l.port}</span>
              <TextInput
                type="number"
                min={1}
                max={65535}
                value={draft.port}
                onChange={(e) => onChange({ port: Math.max(1, Math.min(65535, Number(e.target.value) || 11435)) })}
              />
            </label>
          </div>
          {requiresLoopback && (
            <p role="alert" className="text-sm text-amber-700 dark:text-amber-300">
              {l.loopbackOnlyHint}
            </p>
          )}
          <details className="router-setting">
            <summary>
              {l.apiCompatibility}
              <ChevronRight size={15} className="router-chevron" />
            </summary>
            <section className="space-y-2 px-4 pb-2" aria-label={l.apiCompatibility}>
              <p className="router-hint">{l.apiCompatibilityDesc}</p>
              <dl className="grid gap-1 text-sm">
                {[
                  [l.openAiEndpoint, '/v1/chat/completions'],
                  [l.responsesEndpoint, '/v1/responses'],
                  [l.anthropicEndpoint, '/v1/messages'],
                  [l.tokenCountEndpoint, '/v1/messages/count_tokens'],
                  [l.modelDiscoveryEndpoint, '/v1/models'],
                  [l.slotsEndpoint, '/slots'],
                  [l.readinessEndpoint, '/ready'],
                  [l.metricsEndpoint, '/metrics'],
                ].map(([label, path]) => (
                  <div key={path} className="flex flex-wrap items-baseline gap-x-4">
                    <dt className="min-w-48">{label}</dt>
                    <dd className="break-all font-mono text-xs">
                      {endpoint}
                      {path}
                    </dd>
                  </div>
                ))}
              </dl>
              <p className="router-hint">
                {l.anthropicToolsHint} {l.anthropicScopeHint}
              </p>
            </section>
          </details>
        </Group>
        <Group
          title={w.scheduling}
          summary={strategies[draft.routingStrategy as keyof typeof strategies] ?? draft.routingStrategy}
          icon={<GitFork size={16} />}
        >
          <div className="grid max-w-3xl gap-3 sm:grid-cols-2">
            <label className="min-w-0">
              <span className="router-label">{l.defaultTarget}</span>
              <SelectInput
                className="w-full"
                value={draft.defaultInstanceId}
                onChange={(e) => onChange({ defaultInstanceId: e.target.value })}
              >
                <option value="">{l.noDefault}</option>
                {targets.map((t) => (
                  <option key={t.instanceId} value={t.instanceId}>
                    {t.name} · {t.status === 'running' ? l.running : t.status === 'stopped' ? l.stopped : l.unknown}
                  </option>
                ))}
              </SelectInput>
            </label>
            <label>
              <span className="router-label">{l.routingStrategy}</span>
              <SelectInput
                className="w-full"
                value={draft.routingStrategy}
                onChange={(e) => onChange({ routingStrategy: e.target.value })}
              >
                {Object.entries(strategies).map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </SelectInput>
            </label>
          </div>
          <p className="router-hint">{l.defaultTargetHint}</p>
          <label className="flex items-center gap-3 text-sm">
            <input
              type="checkbox"
              className="router-switch"
              role="switch"
              checked={draft.strictModelRouting}
              onChange={(e) => onChange({ strictModelRouting: e.target.checked })}
            />
            {l.strictRouting}
          </label>
          <p className="router-hint">{l.strictRoutingHint}</p>
        </Group>
        <Group
          title={w.traffic}
          summary={`${l.maxConcurrentRequests} · ${draft.maxConcurrentRequests}`}
          icon={<Activity size={16} />}
        >
          <p className="router-hint">{l.resilienceHint}</p>
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {fields.map(([key, title, min]) => (
              <label key={key}>
                <span className="router-label">{title}</span>
                <TextInput
                  type="number"
                  min={min}
                  value={draft[key]}
                  onChange={(e) => onChange({ [key]: Math.max(min, Number(e.target.value) || min) })}
                />
              </label>
            ))}
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={draft.fairQueueEnabled}
              onChange={(e) => onChange({ fairQueueEnabled: e.target.checked })}
            />
            {m.fairQueue}
          </label>
          <p className="router-hint">{m.fairHint}</p>
        </Group>
        <Group
          title={w.origins}
          summary={originCount ? originCount.toLocaleString(lang) : w.noOrigins}
          icon={<Globe size={16} />}
        >
          <label className="block">
            <span className="router-label">{l.corsOrigins}</span>
            <TextInput
              aria-label={l.corsOrigins}
              placeholder="https://app.example.com, https://admin.example.com"
              value={draft.corsAllowedOrigins.join(',')}
              onChange={(e) => onChange({ corsAllowedOrigins: e.target.value.split(',') })}
            />
          </label>
          <p className="router-hint">{l.corsOriginsHint}</p>
          <p className="router-hint">{l.apiKeyRelationshipHint}</p>
        </Group>
        <Group
          title={l.keepAliveTitle}
          summary={draft.runtimeServiceEnabled ? l.enabled : l.disabled}
          icon={<Server size={16} />}
        >
          <label className="flex items-center gap-3 text-sm">
            <input
              type="checkbox"
              role="switch"
              className="router-switch"
              checked={draft.runtimeServiceEnabled}
              onChange={(e) => onChange({ runtimeServiceEnabled: e.target.checked })}
            />
            {l.keepAliveTitle}
          </label>
          <p className="router-hint">{l.keepAliveDesc}</p>
          <dl className="flex flex-wrap gap-x-6 gap-y-2 text-sm">
            {[
              [l.runtimeProcess, runtimeFresh && runtime.servicePid > 0 ? `PID ${runtime.servicePid}` : l.unavailable],
              [
                l.loginRecovery,
                !runtimeFresh ? l.unknown : runtime.registeredForLogin ? l.registered : l.notRegistered,
              ],
              [l.managedInstances, runtimeFresh ? runtime.managedInstances : '—'],
            ].map(([name, value]) => (
              <div key={name}>
                <dt className="router-muted">{name}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
          {(draft.runtimeServiceEnabled !== config.runtimeServiceEnabled ||
            config.runtimeServiceEnabled !== runtime.backgroundEnabled) && (
            <p className="router-hint">{l.runtimeSyncPending}</p>
          )}
          {runtime.lastError && (
            <p role="alert" className="text-sm text-rose-700 dark:text-rose-300">
              {l.runtimeLastError} · {runtime.lastError}
            </p>
          )}
          {(config.runtimeServiceEnabled || runtime.backgroundEnabled || runtime.managedInstances > 0 || running) && (
            <div className="router-divider">
              <p className="mb-2 text-xs">{l.stopRuntimeDesc}</p>
              <Button variant="danger" size="sm" icon={<PowerOff size={15} />} disabled={busy} onClick={onStopRuntime}>
                {l.stopRuntimeAction}
              </Button>
            </div>
          )}
        </Group>
      </div>
    </section>
  )
}
