import { useEffect, useMemo, useState } from 'react'
import { Download, RefreshCw, Trash2 } from 'lucide-react'
import { invokeApp } from '../lib/ipc'
import { useI18n } from '../i18n'
import { getRouterUsageLabels } from '../i18n/routerUsage'
import { Button, MetricCard, SelectInput, Surface, TextInput } from './ui'

type Summary = {
  requests: number; forwarded: number; success: number; failed: number; rejected: number; cancelled: number; incomplete: number
  complete: number; partial: number; unknown: number; notApplicable: number
  input: number; output: number; cached: number; cacheInput: number; cacheKnown: number; inputKnown: number; outputKnown: number
  items: number; durationMs: number; queueMs: number; firstOutputMs: number; firstOutputCount: number; lastUsed: number
}
type Group = { id: string; name: string; summary: Summary }
type UsageRecord = {
  requestId: string; keyId: string; keyName: string; model: string; instanceId: string; endpoint: string; kind: string
  startedAt: number; completedAt: number; httpStatus: number; forwarded: boolean; outcome: string; quality: string; source: string
  tokens: { input: number | null; output: number | null; cached: number | null; cacheWrite: number | null; reasoning: number | null }
  durationMs: number; queueMs: number; firstOutputMs: number | null; finishReason: string | null; items: number | null
}
type Report = {
  summary: Summary; keys: Group[]; models: Group[]; instances: Group[]; days: Group[]; endpoints: Group[]
  recent: UsageRecord[]; recentTruncated: boolean; droppedRecords: number; writeErrors: number; lastWriteError: string | null
  updatedAt: number; detailDays: number; summaryDays: number
}
type Grouping = 'keys' | 'models' | 'instances' | 'endpoints'
const DAY = 86_400_000
const utcDate = (time: number) => new Date(time).toISOString().slice(0, 10)
const cell = 'whitespace-nowrap px-3 py-3 text-left align-top'

function csvCell(value: string | number) {
  const text = String(value)
  // Keep spreadsheet formula injection out of exported caller-controlled names.
  return `"${(/^[=+@\-\t\r\n]/.test(text) ? `'${text}` : text).replace(/"/g, '""')}"`
}

export default function ProxyUsagePanel() {
  const { lang } = useI18n()
  const l = useMemo(() => getRouterUsageLabels(lang), [lang])
  const [from, setFrom] = useState(() => utcDate(Date.now() - 6 * DAY))
  const [to, setTo] = useState(() => utcDate(Date.now()))
  const [keyId, setKeyId] = useState('')
  const [model, setModel] = useState('')
  const [instanceId, setInstanceId] = useState('')
  const [endpoint, setEndpoint] = useState('')
  const [kind, setKind] = useState('')
  const [grouping, setGrouping] = useState<Grouping>('keys')
  const [report, setReport] = useState<Report | null>(null)
  const [catalog, setCatalog] = useState<Report | null>(null)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const [revision, setRevision] = useState(0)
  const [confirmClear, setConfirmClear] = useState(false)
  const [clearing, setClearing] = useState(false)
  const query = useMemo(() => ({ from: Date.parse(`${from}T00:00:00Z`), to: Date.parse(`${to}T00:00:00Z`) + DAY,
    keyId: keyId || null, model: model || null, instanceId: instanceId || null, endpoint: endpoint || null, kind: kind || null }),
  [from, to, keyId, model, instanceId, endpoint, kind])
  const valid = Number.isFinite(query.from) && Number.isFinite(query.to) && query.to > query.from && query.to - query.from <= 365 * DAY

  useEffect(() => {
    let disposed = false
    let inFlight = false
    setReport(null)
    setConfirmClear(false)
    if (!valid) { setError(l.invalidRange); return }
    const load = async () => {
      if (inFlight || disposed) return
      inFlight = true
      setLoading(true)
      try {
        const base = { ...query, keyId: null, model: null, instanceId: null, endpoint: null }
        const [next, options] = await Promise.all([
          invokeApp<Report>('get_router_usage', { query }),
          invokeApp<Report>('get_router_usage', { query: base }),
        ])
        if (!disposed) { setReport(next); setCatalog(options); setError('') }
      } catch (e) {
        if (!disposed) { setError(String(e)); setReport(null) }
      } finally {
        inFlight = false
        if (!disposed) setLoading(false)
      }
    }
    void load()
    const timer = window.setInterval(() => { if (!document.hidden) void load() }, 15_000)
    return () => { disposed = true; window.clearInterval(timer) }
  }, [query, valid, revision, l.invalidRange])

  const number = (n: number | null | undefined) => n == null ? '—' : n.toLocaleString(lang)
  const percent = (n: number, d: number) => d ? `${(n / d * 100).toFixed(1)}%` : '—'
  const time = (n: number) => n ? new Date(n).toLocaleString(lang) : '—'
  const name = (id: string, label: string) => id === 'anonymous' ? l.anonymous : id === 'unauthenticated' ? l.unauthenticated : label || id || l.unknownName
  const label = (value: string) => (l as Record<string, string>)[value] || value
  const groups = useMemo(() => [...(report?.[grouping] || [])].sort((a, b) => b.summary.requests - a.summary.requests || a.id.localeCompare(b.id)), [report, grouping])
  const summary = report?.summary
  const eligible = summary ? summary.complete + summary.partial + summary.unknown : 0
  const days = [...(report?.days || [])].sort((a, b) => Number(a.id) - Number(b.id))
  const peak = Math.max(1, ...days.map(d => d.summary.input + d.summary.output))
  const range = (count: number) => { setFrom(utcDate(Date.now() - (count - 1) * DAY)); setTo(utcDate(Date.now())) }

  const exportSummary = () => {
    if (!report) return
    const rows: (string | number)[][] = [[l.from, l.to, l.group, 'ID', l.key, l.requests, l.forwarded, l.success, l.failed, l.rejected, l.cancelled, l.incomplete,
      l.input, l.output, l.cached, l.knownInput, l.knownOutput, l.complete, l.partial, l.unknown, l.lastUsed]]
    for (const group of groups) {
      const s = group.summary
      rows.push([from, to, grouping, group.id, name(group.id, group.name), s.requests, s.forwarded, s.success, s.failed, s.rejected, s.cancelled, s.incomplete,
        s.inputKnown ? s.input : '', s.outputKnown ? s.output : '', s.cacheKnown ? s.cached : '', s.inputKnown, s.outputKnown, s.complete, s.partial, s.unknown, new Date(s.lastUsed).toISOString()])
    }
    const url = URL.createObjectURL(new Blob(['\uFEFF', rows.map(row => row.map(csvCell).join(',')).join('\r\n')], { type: 'text/csv;charset=utf-8' }))
    const anchor = document.createElement('a')
    anchor.href = url; anchor.download = `router-usage-${from}-${to}-${grouping}.csv`; anchor.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 1000)
  }
  const clear = async () => {
    setClearing(true)
    try {
      await invokeApp('clear_router_usage', { from: query.from, to: query.to })
      setConfirmClear(false); setRevision(r => r + 1)
    } catch (e) { setError(String(e)) } finally { setClearing(false) }
  }

  return <div className="space-y-5" data-testid="router-usage-panel">
    <Surface as="section" className="p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div><h2 className="text-xl font-semibold">{l.title}</h2><p className="mt-1 text-sm text-slate-500 dark:text-slate-400">{l.description}</p></div>
        <div className="flex flex-wrap gap-2">
          <Button onClick={() => setRevision(r => r + 1)} disabled={loading || !valid} icon={<RefreshCw className="h-4 w-4" />}>{l.refresh}</Button>
          <Button onClick={exportSummary} disabled={!report || !groups.length} icon={<Download className="h-4 w-4" />}>{l.export}</Button>
        </div>
      </div>
      <div className="mt-5 flex flex-wrap items-end gap-3">
        <label className="text-xs">{l.from}<TextInput className="mt-1 block" aria-label={l.from} type="date" value={from} onChange={e => setFrom(e.target.value)} /></label>
        <label className="text-xs">{l.to}<TextInput className="mt-1 block" aria-label={l.to} type="date" value={to} onChange={e => setTo(e.target.value)} /></label>
        <Button onClick={() => range(1)}>{l.today}</Button><Button onClick={() => range(7)}>{l.week}</Button><Button onClick={() => range(30)}>{l.month}</Button>
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
        {([
          [l.key, keyId, setKeyId, catalog?.keys || []], [l.model, model, setModel, catalog?.models || []],
          [l.instance, instanceId, setInstanceId, catalog?.instances || []], [l.endpoint, endpoint, setEndpoint, catalog?.endpoints || []],
        ] as const).map(([title, value, set, options]) => <label key={title} className="min-w-0 text-xs">{title}
          <SelectInput aria-label={`${l.title} ${title}`} className="mt-1 w-full" value={value} onChange={e => set(e.target.value)}>
            <option value="">{l.all}</option>{value && !options.some(o => o.id === value) ? <option value={value}>{value}</option> : null}
            {options.filter(o => o.id).map(o => <option key={o.id} value={o.id}>{name(o.id, o.name)} · {o.id}</option>)}
          </SelectInput>
        </label>)}
        <label className="text-xs">{l.kind}<SelectInput aria-label={l.kind} className="mt-1 w-full" value={kind} onChange={e => setKind(e.target.value)}>
          <option value="">{l.business}</option>{['generation', 'embedding', 'rerank', 'count'].map(k => <option key={k} value={k}>{label(k)}</option>)}
        </SelectInput></label>
      </div>
      <p className="mt-4 text-xs leading-5 text-slate-500 dark:text-slate-400">{l.retention}</p>
      <p className="mt-1 text-xs leading-5 text-slate-500 dark:text-slate-400">{l.accounting}</p>
    </Surface>
    {error ? <Surface className="p-4 text-sm text-red-700 dark:text-red-300" role="alert">{error}</Surface> : null}
    {report && (report.droppedRecords || report.writeErrors || report.lastWriteError) ? <Surface className="p-4 text-sm text-amber-700 dark:text-amber-300" role="status">
      {l.storageWarning} {l.dropped}: {number(report.droppedRecords)} · {l.errors}: {number(report.writeErrors)}
      {report.lastWriteError ? <p className="mt-2 break-words">{report.lastWriteError}</p> : null}
    </Surface> : null}
    {!report && loading ? <p role="status" className="text-sm text-slate-500">{l.loading}</p> : null}
    {summary ? <>
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <MetricCard label={l.requests} value={number(summary.requests)} />
        <MetricCard label={l.input} value={summary.inputKnown ? number(summary.input) : '—'} />
        <MetricCard label={l.output} value={summary.outputKnown ? number(summary.output) : '—'} />
        <MetricCard label={l.coverage} value={percent(summary.complete, eligible)} />
      </div>
      <Surface className="p-4 text-sm">
        <div className="flex flex-wrap gap-x-5 gap-y-2">
          <span>{l.success}: {number(summary.success)}</span><span>{l.failed}: {number(summary.failed)}</span>
          <span>{l.rejected}: {number(summary.rejected)}</span><span>{l.cancelled}: {number(summary.cancelled)}</span>
          <span>{l.incomplete}: {number(summary.incomplete)}</span><span>{l.partial}: {number(summary.partial)}</span>
          <span>{l.unknown}: {number(summary.unknown)}</span><span>{l.cacheRatio}: {percent(summary.cached, summary.cacheInput)}</span>
        </div>
        <p className="mt-3 text-xs leading-5 text-slate-500 dark:text-slate-400">{l.qualityHint}</p>
      </Surface>
      <Surface as="section" className="p-5">
        <div className="mb-4 flex items-center justify-between gap-3"><h3 className="font-semibold">{l.group}</h3>
          <SelectInput aria-label={l.group} value={grouping} onChange={e => setGrouping(e.target.value as Grouping)}>
            <option value="keys">{l.key}</option><option value="models">{l.model}</option><option value="instances">{l.instance}</option><option value="endpoints">{l.endpoint}</option>
          </SelectInput>
        </div>
        {!groups.length ? <p className="py-6 text-sm text-slate-500">{l.empty}</p> : <div className="max-h-[440px] overflow-auto"><table className="w-full text-sm">
          <thead className="sticky top-0 bg-slate-100 dark:bg-slate-800"><tr>{[l.group, l.requests, l.success, l.failed, l.input, l.output, l.cached, l.coverage, l.lastUsed].map(h => <th key={h} className={cell}>{h}</th>)}</tr></thead>
          <tbody>{groups.map(g => <tr key={g.id} className="border-b border-slate-200 dark:border-slate-800">
            <td className={cell}><span className="block max-w-64 truncate" title={g.name}>{name(g.id, g.name)}</span><span className="block max-w-64 truncate text-xs text-slate-500" title={g.id}>{g.id}</span></td>
            <td className={cell}>{number(g.summary.requests)}</td><td className={cell}>{number(g.summary.success)}</td><td className={cell}>{number(g.summary.failed)}</td>
            <td className={cell}>{g.summary.inputKnown ? number(g.summary.input) : '—'}</td><td className={cell}>{g.summary.outputKnown ? number(g.summary.output) : '—'}</td>
            <td className={cell}>{g.summary.cacheKnown ? number(g.summary.cached) : '—'}</td><td className={cell}>{percent(g.summary.complete, g.summary.complete + g.summary.partial + g.summary.unknown)}</td><td className={cell}>{time(g.summary.lastUsed)}</td>
          </tr>)}</tbody>
        </table></div>}
      </Surface>
      <Surface as="section" className="p-5"><h3 className="mb-4 font-semibold">{l.trend}</h3>
        <div className="max-h-80 space-y-3 overflow-auto">{days.length ? days.map(d => <div key={d.id} className="grid grid-cols-[90px_minmax(40px,1fr)_100px] items-center gap-3 text-xs">
          <span>{utcDate(Number(d.id))}</span><div className="h-3 rounded bg-slate-100 dark:bg-slate-800" title={`${l.input}: ${number(d.summary.input)} · ${l.output}: ${number(d.summary.output)}`}>
            <div className="h-3 rounded bg-blue-500" style={{ width: `${(d.summary.input + d.summary.output) / peak * 100}%` }} />
          </div><span className="text-right">{d.summary.inputKnown || d.summary.outputKnown ? number(d.summary.input + d.summary.output) : '—'} Token</span>
        </div>) : <p className="text-sm text-slate-500">{l.empty}</p>}</div>
      </Surface>
      <Surface as="section" className="p-5"><h3 className="font-semibold">{l.details}</h3><p className="mt-2 text-xs text-slate-500 dark:text-slate-400">{l.detailLimit}</p>
        <div className="mt-4 max-h-[440px] overflow-auto"><table className="w-full text-xs">
          <thead className="sticky top-0 bg-slate-100 dark:bg-slate-800"><tr>{[l.lastUsed, l.key, l.model, l.outcome, l.quality, l.input, l.output, l.duration, l.queue, l.firstOutput].map(h => <th key={h} className={cell}>{h}</th>)}</tr></thead>
          <tbody>{report.recent.map(r => <tr key={r.requestId} className="border-b border-slate-200 dark:border-slate-800" title={`${r.requestId}\n${r.endpoint}\n${r.instanceId}\n${r.finishReason || ''}`}>
            <td className={cell}>{time(r.completedAt)}</td><td className={cell}>{name(r.keyId, r.keyName)}</td><td className={`${cell} max-w-48 truncate`}>{r.model || '—'}</td>
            <td className={cell}>{label(r.outcome)} {r.httpStatus || ''}</td><td className={cell}>{label(r.quality)}</td>
            <td className={cell}>{number(r.tokens.input)}</td><td className={cell}>{number(r.tokens.output)}</td>
            <td className={cell}>{number(r.durationMs)} ms</td><td className={cell}>{number(r.queueMs)} ms</td><td className={cell}>{r.firstOutputMs == null ? '—' : `${number(r.firstOutputMs)} ms`}</td>
          </tr>)}</tbody>
        </table></div>
      </Surface>
      <Surface className="p-4">
        {!confirmClear ? <Button onClick={() => setConfirmClear(true)} disabled={!valid || !summary.requests} icon={<Trash2 className="h-4 w-4" />}>{l.clear}</Button>
          : <div role="alert"><p className="text-sm">{l.clearConfirm}</p><div className="mt-3 flex gap-2">
            <Button variant="danger" onClick={() => void clear()} disabled={clearing}>{l.confirm}</Button><Button onClick={() => setConfirmClear(false)} disabled={clearing}>{l.cancel}</Button>
          </div></div>}
      </Surface>
    </> : null}
  </div>
}
