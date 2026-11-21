import { useEffect, useRef, useState } from 'react'
import { Copy, Eye, EyeOff, KeyRound, RefreshCw, Trash2, X } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getProxyLabels } from '../../i18n/pageLabels'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { getRoutingWorkspaceLabels } from '../../i18n/routingWorkspace'
import { Button, IconButton, TextInput } from '../ui'
import { QuotaSettings } from '../routerUsage/QuotaSettings'
import { isStoredApiKey, type ProxyApiKey } from './model'

export type KeySession = { original: ProxyApiKey; value: ProxyApiKey; isNew: boolean }
export const generateCredential = () => `lsm_${crypto.randomUUID().replace(/-/g, '')}`

function NewCredential({ value, change }: { value: string; change: (key: string) => void }) {
  const { lang } = useI18n()
  const l = getProxyLabels(lang),
    w = getRoutingWorkspaceLabels(lang)
  const [visible, setVisible] = useState(false)
  const [message, setMessage] = useState('')
  useEffect(() => {
    if (!visible) return
    const timer = window.setTimeout(() => setVisible(false), 10_000)
    return () => window.clearTimeout(timer)
  }, [visible])
  async function copy() {
    try {
      await navigator.clipboard.writeText(value)
      setMessage(l.apiKeyCopied)
    } catch {
      setMessage(w.clipboardError)
    }
  }
  return (
    <div>
      <label className="router-label" htmlFor="router-new-secret">
        {l.apiKeyValue}
      </label>
      <div className="flex min-w-0 items-center gap-1">
        <TextInput
          id="router-new-secret"
          className="h-9 min-w-0 flex-1 font-mono"
          type={visible ? 'text' : 'password'}
          autoComplete="off"
          spellCheck={false}
          value={value}
          onChange={(event) => {
            change(event.target.value)
            setMessage('')
          }}
        />
        <IconButton
          label={visible ? l.hideApiKey : l.revealApiKey}
          icon={visible ? <EyeOff size={15} /> : <Eye size={15} />}
          onClick={() => setVisible(!visible)}
        />
        <IconButton label={l.copyApiKey} icon={<Copy size={15} />} disabled={!value} onClick={() => void copy()} />
      </div>
      <p className="router-hint" role="status">
        {message || w.copyBeforeSave}
      </p>
    </div>
  )
}

type Props = {
  session: KeySession
  busy: boolean
  dirty: boolean
  lastKey: boolean
  discardPending: boolean
  onPatch: (patch: Partial<ProxyApiKey>) => void
  onClose: () => void
  onDiscard: () => void
  onKeep: () => void
  onSave: (remove: boolean) => void
}

export default function ApiKeyEditor({
  session,
  busy,
  dirty,
  lastKey,
  discardPending,
  onPatch,
  onClose,
  onDiscard,
  onKeep,
  onSave,
}: Props) {
  const { lang } = useI18n()
  const l = getProxyLabels(lang),
    w = getRoutingWorkspaceLabels(lang),
    m = getRouterManagementLabels(lang)
  const { original, value, isNew } = session
  const [tab, setTab] = useState<'general' | 'quota' | 'credential'>('general')
  const [confirmDelete, setConfirmDelete] = useState(false)
  const nameInput = useRef<HTMLInputElement>(null)
  useEffect(() => {
    const input = nameInput.current
    if (!input) return
    input.focus({ preventScroll: true })
    const workspace = input.closest('.router-workspace')
    if (workspace && workspace.clientWidth <= 760) input.closest('.router-editor')?.scrollIntoView({ block: 'start' })
  }, [])
  const rotating = !isNew && value.key !== original.key
  function validate() {
    if (!value.name.trim() && (isNew || original.name.trim())) return w.nameRequired
    const secret = value.key.trim()
    if (isNew || rotating) {
      if (isStoredApiKey(secret)) return w.credentialInvalid
      if (secret.length < 16) return w.credentialLength
    }
    return value.enabled && secret.length < 16 ? l.apiKeyValidation : ''
  }
  const validation = validate()
  function numeric(
    key: 'requestsPerMinute' | 'maxConcurrentRequests' | 'dailyTokenBudget' | 'monthlyTokenBudget',
    title: string,
    max: number,
  ) {
    return (
      <label className="min-w-0">
        <span className="router-label">{title}</span>
        <TextInput
          className="h-9 tabular-nums"
          type="number"
          min={0}
          max={max}
          step={1}
          value={value[key]}
          onChange={(e) => onPatch({ [key]: Math.min(max, Math.max(0, Math.floor(Number(e.target.value) || 0))) })}
        />
      </label>
    )
  }
  return (
    <section
      className="router-editor router-panel"
      aria-labelledby="router-key-editor-title"
      onKeyDown={(e) => {
        if (e.key === 'Escape' && !busy) {
          e.stopPropagation()
          if (discardPending) onKeep()
          else onClose()
        }
      }}
    >
      <header className="flex items-center justify-between gap-3 px-4 pb-1 pt-3">
        <h3 id="router-key-editor-title" className="flex items-center gap-2 font-semibold">
          <KeyRound size={17} className="text-blue-500" />
          {isNew ? l.addApiKey : w.editKey}
        </h3>
        <IconButton label={l.cancel} icon={<X size={16} />} disabled={busy} onClick={onClose} />
      </header>
      <nav className="router-editor-tabs" aria-label={w.editKey}>
        {(['general', 'quota', ...(!isNew ? ['credential' as const] : [])] as const).map((key) => (
          <button
            key={key}
            type="button"
            aria-pressed={tab === key}
            disabled={busy}
            onClick={() => {
              setTab(key)
              setConfirmDelete(false)
            }}
          >
            {key === 'general' ? w.general : key === 'quota' ? w.quotas : w.credential}
          </button>
        ))}
      </nav>
      <form
        className="router-editor-form"
        onSubmit={(e) => {
          e.preventDefault()
          if (!busy && !validation && !discardPending && (isNew || dirty)) onSave(false)
        }}
      >
        <fieldset disabled={busy || discardPending} className="router-editor-fields">
          {tab === 'general' && (
            <>
              <label>
                <span className="router-label">{l.apiKeyName}</span>
                <input
                  ref={nameInput}
                  className="router-input"
                  value={value.name}
                  autoComplete="off"
                  onChange={(e) => onPatch({ name: e.target.value })}
                />
              </label>
              {isNew && <NewCredential value={value.key} change={(key) => onPatch({ key })} />}
              <label className="flex items-center justify-between gap-2 text-sm">
                {w.toggleKey}
                <input
                  type="checkbox"
                  className="router-switch"
                  role="switch"
                  checked={value.enabled}
                  onChange={(e) => onPatch({ enabled: e.target.checked })}
                />
              </label>
              <fieldset>
                <legend className="router-label">{l.scopes}</legend>
                <div className="flex flex-wrap gap-x-4 gap-y-2">
                  {(['inference', 'discovery'] as const).map((scope) => (
                    <label key={scope} className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={value.scopes.includes(scope)}
                        onChange={(e) =>
                          onPatch({
                            scopes: e.target.checked
                              ? [...value.scopes, scope]
                              : value.scopes.filter((s) => s !== scope),
                          })
                        }
                      />
                      {scope === 'inference' ? l.inferenceScope : l.discoveryScope}
                    </label>
                  ))}
                </div>
              </fieldset>
              <div className="router-divider">
                <div className="grid grid-cols-2 gap-3">
                  {numeric('requestsPerMinute', l.apiKeyRequestsPerMinute, 10_000_000)}
                  {numeric('maxConcurrentRequests', m.keyLimit, 100_000)}
                </div>
                <p className="router-hint">{w.limitsHint}</p>
              </div>
            </>
          )}
          {tab === 'quota' && (
            <>
              <div>
                <div className="grid grid-cols-2 gap-3">
                  {numeric('dailyTokenBudget', m.dayBudget, Number.MAX_SAFE_INTEGER)}
                  {numeric('monthlyTokenBudget', m.monthBudget, Number.MAX_SAFE_INTEGER)}
                </div>
                <p className="router-hint">{w.softHint}</p>
              </div>
              <QuotaSettings value={value} onChange={onPatch} />
            </>
          )}
          {tab === 'credential' && (
            <>
              {rotating ? (
                <>
                  <NewCredential value={value.key} change={(key) => onPatch({ key })} />
                  <p className="router-hint">{w.rotateHint}</p>
                </>
              ) : (
                <div>
                  <p className="mb-2 text-sm">{isStoredApiKey(original.key) ? l.apiKeyHashedHint : w.replaceCredential}</p>
                  <Button
                    size="sm"
                    icon={<RefreshCw size={14} />}
                    onClick={() => onPatch({ key: generateCredential() })}
                  >
                    {w.rotate}
                  </Button>
                </div>
              )}
              <div className="router-divider">
                {confirmDelete ? (
                  <div className="space-y-2">
                    <p role="alert" className="text-sm">
                      {lastKey ? w.removeLast : w.removeHint}
                    </p>
                    <div className="flex flex-wrap gap-2">
                      <Button size="sm" onClick={() => setConfirmDelete(false)}>
                        {l.cancel}
                      </Button>
                      <Button size="sm" variant="danger" onClick={() => onSave(true)}>
                        {w.confirmRemove}
                      </Button>
                    </div>
                  </div>
                ) : (
                  <Button size="sm" variant="danger" icon={<Trash2 size={14} />} onClick={() => setConfirmDelete(true)}>
                    {l.removeApiKey}
                  </Button>
                )}
              </div>
            </>
          )}
        </fieldset>
        {validation && (
          <p className="px-4 pb-2 text-xs text-rose-700 dark:text-rose-300" role="alert">
            {validation}
          </p>
        )}
        <footer className="router-editor-footer">
          {discardPending ? (
            <>
              <p className="w-full text-sm" role="alert">
                {w.discardPrompt}
              </p>
              <Button size="sm" onClick={onDiscard}>
                {w.discard}
              </Button>
              <Button size="sm" variant="primary" onClick={onKeep}>
                {w.keepEditing}
              </Button>
            </>
          ) : (
            <>
              <Button size="sm" disabled={busy} onClick={onClose}>
                {l.cancel}
              </Button>
              <Button
                size="sm"
                type="submit"
                variant="primary"
                disabled={busy || Boolean(validation) || (!dirty && !isNew) || confirmDelete}
              >
                {busy ? l.saving : w.saveKey}
              </Button>
            </>
          )}
        </footer>
      </form>
    </section>
  )
}
