import { useEffect, useMemo, useRef, useState } from 'react'
import { invokeApp } from '../lib/ipc'
import { useI18n } from '../i18n'
import { getRouterManagementLabels } from '../i18n/routerManagement'
import { getRouterUsageLabels } from '../i18n/routerUsage'
import { diagnosticValue, failureCodes, failureText, getRouterDiagnosticsLabels } from '../i18n/routerDiagnostics'
import { Button, SelectInput, Surface, TextInput } from './ui'
import type { UsageRecord } from './ProxyUsagePanel'

type Filters = { from: number; to: number; keyId: string | null; model: string | null; instanceId: string | null; endpoint: string | null; kind: string | null }
type Cursor = { completedAt: number; requestId: string; snapshotRowId: number }
type Page = { records: UsageRecord[]; nextCursor: Cursor | null; firstCursor: Cursor }
const utcTime = (time: number) => new Date(time).toISOString().slice(0, 16)
const cell = 'whitespace-nowrap px-3 py-3 text-left align-top'

export default function ProxyRequestHistory({ query }: { query: Filters }) {
  const { lang } = useI18n()
  const l = useMemo(() => ({ ...getRouterUsageLabels(lang), ...getRouterDiagnosticsLabels(lang) }), [lang])
  const [from, setFrom] = useState(() => utcTime(query.from))
  const [to, setTo] = useState(() => utcTime(query.to))
  const [outcome, setOutcome] = useState('')
  const [failureCode, setFailureCode] = useState('')
  const management = getRouterManagementLabels(lang)
  const [slowMetric, setSlowMetric] = useState('')
  const [threshold, setThreshold] = useState(1000)
  const [requestId, setRequestId] = useState('')
  const [cursors, setCursors] = useState<(Cursor | null)[]>([null])
  const [page, setPage] = useState<Page | null>(null)
  const [selected, setSelected] = useState<UsageRecord | null>(null)
  const [error, setError] = useState('')
  const [copied, setCopied] = useState('')
  const [loading, setLoading] = useState(false)
  const [revision, setRevision] = useState(0)
  const detailRef = useRef<HTMLDivElement>(null)
  useEffect(() => { if (selected) detailRef.current?.scrollIntoView({ block: 'start' }) }, [selected])
  const reset = () => { setCursors([null]); setSelected(null); setCopied('') }
  const cursor = cursors[cursors.length - 1]
  const filters = useMemo(() => ({ ...query, from: Date.parse(`${from}Z`), to: Date.parse(`${to}Z`),
    outcome: outcome || null, failureCode: failureCode || null, requestId: requestId.trim() || null, cursor, minDurationMs: slowMetric === 'duration' ? threshold : null, minQueueMs: slowMetric === 'queue' ? threshold : null, minFirstOutputMs: slowMetric === 'firstOutput' ? threshold : null }), [query, from, to, outcome, failureCode, requestId, cursor, slowMetric, threshold])
  const valid = Number.isFinite(filters.from) && Number.isFinite(filters.to) && filters.from >= query.from && filters.to <= query.to && filters.to > filters.from

  useEffect(() => {
    let disposed = false
    setPage(null)
    if (!valid) { setLoading(false); setError(l.invalidDetailRange); return }
    setLoading(true); setError('')
    const timer = window.setTimeout(() => {
      void invokeApp<Page>('get_router_usage_requests', { query: filters }).then(next => {
        if (!disposed) setPage(next)
      }).catch(e => { if (!disposed) setError(String(e)) }).finally(() => { if (!disposed) setLoading(false) })
    }, 200)
    return () => { disposed = true; window.clearTimeout(timer) }
  }, [filters, valid, revision, l.invalidDetailRange])

  const number = (n: number | null | undefined) => n == null ? '—' : n.toLocaleString(lang)
  const time = (n: number) => new Date(n).toLocaleString(lang)
  const label = (v: string) => (l as Record<string, string>)[v] || diagnosticValue(v, lang)
  const failure = selected?.failure ? failureText(selected.failure.code, lang) : null
  const detailRows: [string, string][] = selected ? [
    [l.requestId, selected.requestId], [l.responseId, [selected.responseRequestId, selected.responseXRequestId].filter((v, i, values) => v && values.indexOf(v) === i).join(' · ') || '—'], [l.upstreamId, selected.upstreamRequestId || '—'],
    [l.key, `${selected.keyName} · ${selected.keyId}`], [l.model, selected.model || '—'], [l.instance, selected.instanceId || '—'], [l.endpoint, selected.endpoint],
    [l.lastUsed, time(selected.completedAt)], [l.outcome, `${label(selected.outcome)} · ${selected.httpStatus || '—'}`],
    [l.stage, selected.failure ? label(selected.failure.stage) : '—'], [l.code, selected.failure?.code || '—'],
    [l.reason, failure?.title || '—'], [l.advice, failure?.advice || '—'],
    [l.quality, label(selected.quality)], [l.source, label(selected.source)],
    [l.input, number(selected.tokens.input)], [l.output, number(selected.tokens.output)], [l.cached, number(selected.tokens.cached)],
    [l.cacheWrite, number(selected.tokens.cacheWrite)], [l.reasoning, number(selected.tokens.reasoning)],
    [l.duration, `${number(selected.durationMs)} ms`], [l.queue, `${number(selected.queueMs)} ms`],
    [l.firstOutput, selected.firstOutputMs == null ? '—' : `${number(selected.firstOutputMs)} ms`], [l.finish, selected.finishReason || '—'], [l.items, number(selected.items)],
  ] : []
  const budget = selected?.contextBudget
  const budgetRows: [string, string][] = budget ? [[l.budgetInput, number(budget.inputTokens)], [l.budgetOutput, number(budget.requestedOutputTokens)],
    [l.window, number(budget.contextWindow)], [l.excess, number(budget.excessTokens)], [l.source, label(budget.inputSource)]] : []
  const copy = async () => {
    try { await navigator.clipboard.writeText([...detailRows, ...budgetRows].map(([k, v]) => `${k}: ${v}`).join('\n')); setCopied(l.copied) }
    catch { setCopied(l.copyFailed) }
  }

  return <Surface as="section" className="p-5" data-testid="router-request-history">
    <h3 className="font-semibold">{l.history}</h3><p className="mt-2 text-xs text-slate-500 dark:text-slate-400">{l.historyHint}</p>
    <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
      <label className="text-xs">{l.startTime}<TextInput type="datetime-local" aria-label={l.startTime} className="mt-1 w-full" value={from} onChange={e => { setFrom(e.target.value); reset() }} /></label>
      <label className="text-xs">{l.endTime}<TextInput type="datetime-local" aria-label={l.endTime} className="mt-1 w-full" value={to} onChange={e => { setTo(e.target.value); reset() }} /></label>
      <label className="text-xs">{l.outcome}<SelectInput aria-label={l.outcome} className="mt-1 w-full" value={outcome} onChange={e => { setOutcome(e.target.value); reset() }}>
        <option value="">{l.all}</option>{['success', 'failed', 'rejected', 'cancelled', 'incomplete'].map(v => <option key={v} value={v}>{label(v)}</option>)}
      </SelectInput></label>
      <label className="text-xs">{l.errorType}<SelectInput aria-label={l.errorType} className="mt-1 w-full" value={failureCode} onChange={e => { setFailureCode(e.target.value); reset() }}>
        <option value="">{l.all}</option>{failureCodes.map(v => <option key={v} value={v}>{failureText(v, lang).title}</option>)}
      </SelectInput></label>
      <label className="text-xs sm:col-span-2">{l.lookup}<TextInput aria-label={l.lookup} placeholder={l.lookupHint} className="mt-1 w-full" value={requestId} maxLength={128} onChange={e => { setRequestId(e.target.value); reset() }} /></label>
    </div>
    <div className="mt-4 flex flex-wrap gap-3"><label className="text-xs">{management.slow}<SelectInput className="ml-2" aria-label={management.slow} value={slowMetric} onChange={e => { setSlowMetric(e.target.value); reset() }}><option value="">{management.all}</option>{(['duration', 'queue', 'firstOutput'] as const).map(m => <option key={m} value={m}>{management[m]}</option>)}</SelectInput></label>{slowMetric ? <label className="text-xs">{management.threshold}<TextInput className="ml-2 w-32" aria-label={management.threshold} type="number" min={0} max={4294967295} value={threshold} onChange={e => { setThreshold(Math.min(4294967295, Math.max(0, Math.floor(Number(e.target.value) || 0)))); reset() }} /></label> : null}</div>
    <div className="mt-4 flex flex-wrap items-center gap-3">
      <Button disabled={loading || cursors.length < 2} onClick={() => { setCursors(c => c.slice(0, -1)); setSelected(null) }}>{l.previous}</Button>
      <span className="text-sm">{l.page} {number(cursors.length)}</span>
      <Button disabled={loading || !page?.nextCursor} onClick={() => { if (page?.nextCursor) setCursors(c => [page.firstCursor, ...c.slice(1), page.nextCursor]); setSelected(null) }}>{l.next}</Button>
      <Button disabled={loading || !valid} onClick={() => { reset(); setRevision(r => r + 1) }}>{l.refresh}</Button>
    </div>
    {error ? <p role="alert" className="mt-4 text-sm text-red-700 dark:text-red-300">{error}</p> : null}
    {loading ? <p role="status" className="mt-4 text-sm">{l.loading}</p> : null}
    {page ? <div className="mt-4 max-h-[440px] overflow-auto"><table className="w-full text-xs">
      <thead className="sticky top-0 bg-slate-100 dark:bg-slate-800"><tr>{[l.lastUsed, l.key, l.model, l.outcome, l.errorType, l.quality, l.view].map(h => <th key={h} className={cell}>{h}</th>)}</tr></thead>
      <tbody>{page.records.map(r => <tr key={r.requestId} className="border-b border-slate-200 dark:border-slate-800">
        <td className={cell}>{time(r.completedAt)}</td><td className={cell}>{r.keyName}</td><td className={`${cell} max-w-48 truncate`}>{r.model || '—'}</td>
        <td className={cell}>{label(r.outcome)} {r.httpStatus || ''}</td><td className={cell}>{r.failure ? failureText(r.failure.code, lang).title : '—'}</td><td className={cell}>{label(r.quality)}</td>
        <td className={cell}><Button aria-label={`${l.view} ${r.requestId}`} onClick={() => { setSelected(r); setCopied('') }}>{l.view}</Button></td>
      </tr>)}</tbody>
    </table>{!page.records.length ? <p className="py-4 text-sm">{l.empty}</p> : null}</div> : null}
    {selected ? <div ref={detailRef} className="mt-5 scroll-mt-24 border-t border-slate-200 pt-5 dark:border-slate-700" data-testid="request-diagnostic-detail">
      <div className="flex flex-wrap items-center justify-between gap-3"><h4 className="font-semibold">{l.view}</h4><div className="flex gap-2"><Button onClick={() => void copy()}>{l.copy}</Button><Button onClick={() => setSelected(null)}>{l.close}</Button></div></div>
      {copied ? <p role="status" className="mt-2 text-sm">{copied}</p> : null}
      {failure ? <div className="mt-4 space-y-2 text-sm"><p>{l.reason}: {failure.title}</p><p>{l.advice}: {failure.advice}</p></div> : null}
      {budget ? <div className="mt-5"><h4 className="font-semibold">{l.budget}</h4><p className="mt-2 text-xs text-slate-500 dark:text-slate-400">{l.budgetHint}</p>
        <dl className="mt-3 grid gap-3 sm:grid-cols-2">{budgetRows.map(([k, v]) => <div key={k}><dt className="text-xs text-slate-500 dark:text-slate-400">{k}</dt><dd className="mt-1 text-sm">{v}</dd></div>)}</dl>
      </div> : null}
      {!selected.forwarded && selected.kind === 'generation' ? <p className="mt-4 text-sm">{l.noGeneration}</p> : null}
      <dl className="mt-4 grid gap-x-6 gap-y-3 sm:grid-cols-2">{detailRows.filter(([k]) => k !== l.reason && k !== l.advice).map(([k, v]) => <div key={k} className="min-w-0"><dt className="text-xs text-slate-500 dark:text-slate-400">{k}</dt><dd className="mt-1 break-words text-sm">{v}</dd></div>)}</dl>
    </div> : null}
  </Surface>
}
