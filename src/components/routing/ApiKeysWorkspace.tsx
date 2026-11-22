import { useRef, useState } from 'react'
import { ChevronLeft, ChevronRight, Plus, Search } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getProxyLabels } from '../../i18n/pageLabels'
import { getRoutingWorkspaceLabels } from '../../i18n/routingWorkspace'
import { Button, TextInput } from '../ui'
import ApiKeyEditor, { generateCredential, type KeySession } from './ApiKeyEditor'
import type { ProxyApiKey } from './model'

const PAGE_SIZE = 8
type Filter = 'all' | 'enabled' | 'disabled'
type Props = {
  keys: ProxyApiKey[]
  legacy: boolean
  busy: boolean
  active: boolean
  available: boolean
  onCommit: (original: ProxyApiKey | null, next: ProxyApiKey | null) => Promise<boolean>
}

export default function ApiKeysWorkspace({ keys, legacy, busy, active, available, onCommit }: Props) {
  const { lang } = useI18n()
  const l = getProxyLabels(lang),
    w = getRoutingWorkspaceLabels(lang)
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<Filter>('all')
  const [page, setPage] = useState(1)
  const [session, setSession] = useState<KeySession | null>(null)
  const [pending, setPending] = useState<{ next: KeySession | null } | null>(null)
  const list = useRef<HTMLDivElement>(null)
  const listScrollTop = useRef(0)
  const dirty = session !== null && JSON.stringify(session.original) !== JSON.stringify(session.value)
  const needle = query.trim().toLocaleLowerCase()
  const matches = keys.filter((key) => key.name.toLocaleLowerCase().includes(needle))
  const enabled = matches.filter((k) => k.enabled).length
  const counts = { all: matches.length, enabled, disabled: matches.length - enabled }
  const filtered = filter === 'all' ? matches : matches.filter((k) => k.enabled === (filter === 'enabled'))
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE))
  const current = Math.min(page, pages)
  const start = (current - 1) * PAGE_SIZE
  const windowPages = Array.from(new Set([1, current - 1, current, current + 1, pages]))
    .filter((n) => n > 0 && n <= pages)
    .sort((a, b) => a - b)

  function select(next: KeySession | null) {
    if (busy || (session && session.value.id === next?.value.id)) return
    if (dirty) setPending({ next })
    else finishSelect(next)
  }
  function finishSelect(next: KeySession | null) {
    const id = session?.value.id
    const scroll = list.current?.closest<HTMLElement>('[data-workspace-scroll]')
    if (next && !session) listScrollTop.current = scroll?.scrollTop ?? 0
    setSession(next)
    setPending(null)
    if (!next)
      requestAnimationFrame(() => {
        const row = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[data-key-id]') ?? []).find(
          (b) => b.dataset.keyId === id,
        )
        const trigger = row ?? list.current?.querySelector<HTMLButtonElement>('[data-add-key]')
        if (scroll) scroll.scrollTop = listScrollTop.current
        trigger?.focus({ preventScroll: true })
      })
  }
  function add() {
    const key: ProxyApiKey = {
      id: crypto.randomUUID(),
      name: `API Key ${keys.length + 1}`,
      key: generateCredential(),
      enabled: true,
      scopes: ['inference', 'discovery'],
      requestsPerMinute: 0,
      maxConcurrentRequests: 0,
      dailyTokenBudget: 0,
      monthlyTokenBudget: 0,
      dailyTokenLimit: 0,
      monthlyTokenLimit: 0,
      quotaDefaultOutputTokens: 32768,
    }
    select({ isNew: true, original: key, value: key })
  }
  async function commit(remove: boolean) {
    if (!session || busy) return
    const saved = await onCommit(session.isNew ? null : session.original, remove ? null : session.value)
    if (!saved) return
    if (session.isNew) {
      setQuery('')
      setFilter('all')
      setPage(1)
    } else if (remove && filtered.length === start + 1) setPage(Math.max(1, current - 1))
    finishSelect(null)
  }
  const limit = (value: number) =>
    value ? (
      <span className="font-mono tabular-nums">{value.toLocaleString(lang)}</span>
    ) : (
      <span className="router-muted">{w.inherited}</span>
    )
  const authHint = !keys.length
    ? legacy
      ? w.legacyKey
      : l.noApiKeys
    : keys.some((k) => k.enabled)
      ? w.keyHint
      : w.allDisabled
  return (
    <section ref={list} aria-labelledby="router-keys-title" className="router-keys">
      <header className="router-section-header">
        <div>
          <h2 id="router-keys-title">{w.keys}</h2>
          <p>{available ? authHint : l.unavailable}</p>
        </div>
        <Button variant="primary" size="sm" data-add-key icon={<Plus size={16} />} disabled={busy} onClick={add}>
          {l.addApiKey}
        </Button>
      </header>
      <div className={session ? 'router-key-split is-editing' : 'router-key-split'}>
        <div className="router-key-list">
          <div className="router-key-tools">
            <div className="router-filters" role="group" aria-label={l.status}>
              {(['all', 'enabled', 'disabled'] as const).map((item) => (
                <button
                  key={item}
                  type="button"
                  aria-pressed={filter === item}
                  onClick={() => {
                    setFilter(item)
                    setPage(1)
                  }}
                >
                  {item === 'all' ? w.all : item === 'enabled' ? l.enabled : w.disabled}
                  <span className="font-mono text-xs">{counts[item]}</span>
                </button>
              ))}
            </div>
            <TextInput
              type="search"
              aria-label={w.search}
              placeholder={w.search}
              leadingIcon={<Search size={15} />}
              className="router-key-search"
              inputClassName="h-9"
              value={query}
              onChange={(e) => {
                setQuery(e.target.value)
                setPage(1)
              }}
            />
          </div>
          <div className="router-panel overflow-hidden">
            <div className="router-key-rows">
              <table className="router-table router-key-table" aria-label={w.keys}>
                <thead>
                  <tr>
                    <th>{l.apiKeyName}</th>
                    <th className="key-scope">{l.scopes}</th>
                    <th className="key-rate numeric">{w.rate}</th>
                    <th className="key-limit numeric">{w.concurrency}</th>
                    <th>{l.status}</th>
                    <th className="key-arrow">
                      <span className="sr-only">{w.edit}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.slice(start, start + PAGE_SIZE).map((k) => {
                    const scopes =
                      k.scopes
                        .map((s) => (s === 'inference' ? l.inferenceScope : s === 'discovery' ? l.discoveryScope : s))
                        .join(' · ') || '—'
                    const open = () => select({ original: k, value: k, isNew: false })
                    return (
                      <tr key={k.id} className="console-list-row cursor-pointer" data-selected={session?.value.id === k.id} onClick={open}>
                        <td>
                          <button
                            type="button"
                            data-key-id={k.id}
                            aria-label={`${w.edit} ${k.name || w.unnamed}`}
                            title={k.name}
                            disabled={busy}
                            onClick={(e) => {
                              e.stopPropagation()
                              open()
                            }}
                          >
                            {k.name || w.unnamed}
                          </button>
                        </td>
                        <td className="key-scope">{scopes}</td>
                        <td className="key-rate numeric">{limit(k.requestsPerMinute)}</td>
                        <td className="key-limit numeric">{limit(k.maxConcurrentRequests)}</td>
                        <td>
                          <span className="router-key-state" data-enabled={k.enabled}>
                            <span aria-hidden="true" />
                            {k.enabled ? l.enabled : w.disabled}
                          </span>
                        </td>
                        <td className="key-arrow">
                          <ChevronRight size={14} />
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
              {!filtered.length && (
                <div className="router-empty">
                  <p>{available ? (keys.length ? w.noMatches : w.emptyKeys) : l.unavailable}</p>
                  {keys.length > 0 && (
                    <Button
                      size="sm"
                      variant="subtle"
                      onClick={() => {
                        setQuery('')
                        setFilter('all')
                        setPage(1)
                      }}
                    >
                      {w.clear}
                    </Button>
                  )}
                </div>
              )}
            </div>
            <div className="router-pager">
              <span aria-live="polite">
                {w.range(
                  filtered.length ? start + 1 : 0,
                  Math.min(start + PAGE_SIZE, filtered.length),
                  filtered.length,
                )}
              </span>
              <nav aria-label={w.keys}>
                <button
                  type="button"
                  aria-label={w.previous}
                  disabled={current <= 1}
                  onClick={() => setPage(current - 1)}
                >
                  <ChevronLeft size={15} />
                </button>
                {windowPages.map((n, i) => (
                  <span key={n}>
                    {i > 0 && n - windowPages[i - 1] > 1 && <span aria-hidden="true">…</span>}
                    <button
                      type="button"
                      aria-label={w.page(n)}
                      aria-current={n === current ? 'page' : undefined}
                      onClick={() => setPage(n)}
                    >
                      {n}
                    </button>
                  </span>
                ))}
                <button
                  type="button"
                  aria-label={w.next}
                  disabled={current >= pages}
                  onClick={() => setPage(current + 1)}
                >
                  <ChevronRight size={15} />
                </button>
              </nav>
            </div>
          </div>
        </div>
        {active && session && (
          <ApiKeyEditor
            key={session.value.id}
            session={session}
            busy={busy}
            dirty={dirty}
            lastKey={keys.length === 1 && !legacy}
            discardPending={Boolean(pending)}
            onPatch={(patch) => setSession((v) => v && { ...v, value: { ...v.value, ...patch } })}
            onClose={() => select(null)}
            onKeep={() => setPending(null)}
            onDiscard={() => finishSelect(pending?.next ?? null)}
            onSave={(remove) => void commit(remove)}
          />
        )}
      </div>
    </section>
  )
}
