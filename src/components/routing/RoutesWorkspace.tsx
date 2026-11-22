import { useState } from 'react'
import { ChevronRight, Plus, Route, Trash2 } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getProxyLabels } from '../../i18n/pageLabels'
import { getRoutingWorkspaceLabels } from '../../i18n/routingWorkspace'
import { Button, EmptyPanel, SelectInput, StatusBadge, TextInput } from '../ui'
import {
  routeAvailabilityView,
  type ProxyRoute,
  type ProxyTarget,
  type RouteAvailability,
  type RouteIssue,
  type RouteTestView,
} from './model'

type Props = {
  routes: ProxyRoute[]
  targets: ProxyTarget[]
  targetsFresh: boolean
  metrics: Array<[string, string | number]>
  availability: Map<string, RouteAvailability>
  issues: Map<string, RouteIssue>
  tests: Record<string, RouteTestView>
  testingId: string | null
  onAdd: () => string
  onUpdate: (id: string, patch: Partial<ProxyRoute>) => void
  onRemove: (id: string) => void
  onTest: (route: ProxyRoute) => void
}
export default function RoutesWorkspace({
  routes,
  targets,
  targetsFresh,
  metrics,
  availability,
  issues,
  tests,
  testingId,
  onAdd,
  onUpdate,
  onRemove,
  onTest,
}: Props) {
  const { lang } = useI18n()
  const l = getProxyLabels(lang),
    w = getRoutingWorkspaceLabels(lang)
  const [list, setList] = useState<'routes' | 'targets'>('routes')
  const [editing, setEditing] = useState<string | null>(null)
  const selected = routes.find((route) => route.id === editing)
  const issue = selected && issues.get(selected.id)
  const targetNames = new Map(targets.map((t) => [t.instanceId, t.name]))
  return (
    <section aria-labelledby="router-routes-title">
      <header className="router-section-header">
        <div>
          <h2 id="router-routes-title">{w.routes}</h2>
        </div>
        {list === 'routes' && (
          <Button size="sm" variant="primary" icon={<Plus size={16} />} onClick={() => setEditing(onAdd())}>
            {l.addRoute}
          </Button>
        )}
      </header>
      <div className="mb-4 flex flex-wrap gap-x-6 gap-y-2 text-sm">
        {metrics.map(([label, value]) => (
          <div key={label} className="flex items-baseline gap-2">
            <p className="router-muted">{label}</p>
            <p className="font-medium tabular-nums">{value}</p>
          </div>
        ))}
      </div>
      <div className="router-filters mb-3" role="group" aria-label={w.routes}>
        <button type="button" aria-pressed={list === 'routes'} onClick={() => setList('routes')}>
          {l.routeTable}
        </button>
        <button type="button" aria-pressed={list === 'targets'} onClick={() => setList('targets')}>
          {w.targets}
        </button>
      </div>
      {list === 'routes' ? (
        <>
          <section aria-label={l.routeTable} className="router-panel overflow-x-auto">
            {routes.length === 0 ? (
              <EmptyPanel title={l.noRoutes} />
            ) : (
              <table className="router-table" aria-label={l.routeTable}>
                <thead>
                  <tr>
                    <th>{l.modelPattern}</th>
                    <th>{l.target}</th>
                    <th className="router-route-optional">{l.priority}</th>
                    <th>{l.routeAvailability}</th>
                    <th>{l.actions}</th>
                  </tr>
                </thead>
                <tbody>
                  {routes.map((route) => {
                    const kind = availability.get(route.id)?.kind ?? 'pending'
                    const badge = routeAvailabilityView(kind, l),
                      test = tests[route.id]
                    const canTest = !['disabled', 'pending', 'invalid', 'missing'].includes(kind)
                    const name = route.modelAlias || w.unnamed
                    return (
                      <tr key={route.id}>
                        <td>
                          <button
                            type="button"
                            className="break-all text-left font-semibold text-blue-700 dark:text-blue-300"
                            aria-label={`${w.edit} ${name}`}
                            onClick={() => setEditing(route.id)}
                          >
                            {name}
                          </button>
                        </td>
                        <td className="break-words">
                          {targetNames.get(route.targetInstanceId) ||
                            (route.targetInstanceId ? l.routeTargetMissingShort : '—')}
                        </td>
                        <td className="router-route-optional tabular-nums">{route.priority}</td>
                        <td>
                          <StatusBadge tone={badge.tone}>{badge.label}</StatusBadge>
                          {test && (
                            <p role={test.tone === 'red' ? 'alert' : 'status'} className="router-hint">
                              {test.message}
                            </p>
                          )}
                        </td>
                        <td>
                          <Button
                            size="sm"
                            variant="subtle"
                            disabled={!canTest || testingId !== null}
                            title={canTest ? l.routeTestHint : l.routeTestSaveFirst}
                            icon={<Route size={14} />}
                            onClick={() => onTest(route)}
                          >
                            {testingId === route.id ? l.testingRoute : l.testRoute}
                          </Button>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            )}
          </section>
          {selected && (
            <section className="router-panel mt-3 space-y-3 p-4" aria-label={w.routeEditor}>
              <header className="flex items-center justify-between gap-2">
                <h3 className="font-semibold">{w.routeEditor}</h3>
                <label className="flex items-center gap-2 text-xs">
                  <input
                    type="checkbox"
                    role="switch"
                    className="router-switch"
                    aria-label={l.routeStatusControl}
                    checked={selected.enabled}
                    onChange={(e) => onUpdate(selected.id, { enabled: e.target.checked })}
                  />
                  {selected.enabled ? l.enabled : l.disabled}
                </label>
              </header>
              <div className="grid gap-3 sm:grid-cols-2">
                <label>
                  <span className="router-label">{l.modelPattern}</span>
                  <TextInput
                    value={selected.modelAlias}
                    aria-label={l.modelPattern}
                    aria-invalid={Boolean(issue?.modelAlias)}
                    onChange={(e) => onUpdate(selected.id, { modelAlias: e.target.value })}
                  />
                  {issue?.modelAlias && (
                    <span role="alert" className="text-xs text-rose-600 dark:text-rose-300">
                      {issue.modelAlias}
                    </span>
                  )}
                </label>
                <label>
                  <span className="router-label">{l.target}</span>
                  <SelectInput
                    className="w-full"
                    value={selected.targetInstanceId}
                    aria-label={l.target}
                    aria-invalid={Boolean(issue?.targetInstanceId)}
                    onChange={(e) => onUpdate(selected.id, { targetInstanceId: e.target.value })}
                  >
                    <option value="">{l.selectTarget}</option>
                    {targets.map((t) => (
                      <option key={t.instanceId} value={t.instanceId}>
                        {t.name} · {t.status === 'running' ? l.running : t.status === 'stopped' ? l.stopped : l.unknown}
                      </option>
                    ))}
                  </SelectInput>
                  {issue?.targetInstanceId && (
                    <span role="alert" className="text-xs text-rose-600 dark:text-rose-300">
                      {issue.targetInstanceId}
                    </span>
                  )}
                </label>
              </div>
              <div className="grid gap-3 sm:grid-cols-3">
                {(
                  [
                    ['priority', l.priority],
                    ['weight', l.weight],
                    ['maxConcurrentRequests', l.routeConcurrency],
                  ] as const
                ).map(([field, title]) => (
                  <label key={field}>
                    <span className="router-label">{title}</span>
                    <TextInput
                      type="number"
                      min={field === 'weight' ? 1 : field === 'priority' ? undefined : 0}
                      value={selected[field]}
                      onChange={(e) => {
                        const n = Number(e.target.value) || 0
                        onUpdate(selected.id, {
                          [field]: field === 'priority' ? n : Math.max(field === 'weight' ? 1 : 0, n),
                        })
                      }}
                    />
                  </label>
                ))}
              </div>
              <div className="flex justify-between gap-3">
                <Button
                  variant="danger"
                  size="sm"
                  icon={<Trash2 size={14} />}
                  onClick={() => {
                    onRemove(selected.id)
                    setEditing(null)
                  }}
                >
                  {l.removeRoute}
                </Button>
                <Button size="sm" onClick={() => setEditing(null)}>
                  {w.done}
                </Button>
              </div>
            </section>
          )}
          <details className="router-setting mt-2">
            <summary>
              {w.routeHelp}
              <ChevronRight size={14} className="router-chevron" />
            </summary>
            <div className="router-setting-body text-xs leading-5 text-ui-muted">
              <p>{l.routeTableHint}</p>
              <p>{l.routeIdentityHint}</p>
              <p>{l.routeHealthHint}</p>
            </div>
          </details>
        </>
      ) : (
        <div className="router-panel overflow-x-auto">
          {!targetsFresh && (
            <p className="p-3 text-xs text-amber-700 dark:text-amber-300">
              {l.targetList} · {l.unavailable}
            </p>
          )}
          {targets.length === 0 ? (
            <EmptyPanel title={l.noTargets} />
          ) : (
            <table className="router-table" aria-label={w.targets}>
              <thead>
                <tr>
                  <th>{w.targets}</th>
                  <th>{l.endpoint}</th>
                  <th className="router-route-optional">{l.alias}</th>
                  <th>{l.status}</th>
                </tr>
              </thead>
              <tbody>
                {targets.map((t) => (
                  <tr key={t.instanceId}>
                    <td>{t.name}</td>
                    <td className="break-all font-mono text-xs">{t.endpoint}</td>
                    <td className="router-route-optional break-all">{t.alias || '—'}</td>
                    <td>
                      <StatusBadge
                        tone={t.status === 'running' ? 'emerald' : t.status === 'stopped' ? 'slate' : 'amber'}
                      >
                        {t.status === 'running' ? l.running : t.status === 'stopped' ? l.stopped : l.unknown}
                      </StatusBadge>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
    </section>
  )
}
