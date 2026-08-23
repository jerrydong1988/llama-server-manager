import { Ban, FlaskConical, LoaderCircle, ShieldCheck } from 'lucide-react'
import type { EngineInfo, EngineQualificationStatus, ModelInfo } from '../store'
import { normalizeEngineQualificationStatus } from '../engineCapabilities'
import type { getEngineLabels } from '../i18n/pageLabels'
import { formatBytes, formatMs } from './monitoring/monitoringFormat'
import { Badge, Button, InsetSurface, SelectInput } from './ui'

type Labels = ReturnType<typeof getEngineLabels>

type Props = {
  engine: EngineInfo
  models: ModelInfo[]
  selectedModelId: string
  qualifying: boolean
  busy: boolean
  lang: string
  labels: Labels
  onModelChange: (modelId: string) => void
  onQualify: () => void
  onCancel: () => void
}

const eligibleModel = (model: ModelInfo) => (
  model.file_type === 'model'
  && !model.is_shard
  && !model.capabilities?.is_mmproj
  && model.capabilities?.is_embedding_model !== true
  && model.capabilities?.is_reranker_model !== true
)

const statusTone = (status: EngineQualificationStatus): 'slate' | 'emerald' | 'amber' | 'red' => {
  if (status === 'passed') return 'emerald'
  if (status === 'failed' || status === 'cancelled') return 'red'
  if (status === 'incomplete' || status === 'stale') return 'amber'
  return 'slate'
}

const statusLabel = (status: EngineQualificationStatus, labels: Labels) => {
  if (status === 'passed') return labels.qualificationPassed
  if (status === 'failed') return labels.qualificationFailedStatus
  if (status === 'incomplete') return labels.qualificationIncomplete
  if (status === 'cancelled') return labels.qualificationCancelled
  if (status === 'stale') return labels.qualificationStale
  return labels.qualificationUnqualified
}

const checkLabel = (name: string, labels: Labels) => {
  if (name === 'version') return labels.qualificationCheckVersion
  if (name === 'capabilities') return labels.qualificationCheckCapabilities
  if (name === 'startup') return labels.qualificationCheckStartup
  if (name === 'health') return labels.qualificationCheckHealth
  if (name === 'inference') return labels.qualificationCheckInference
  return name
}

const checkStatusLabel = (status: string, labels: Labels) => {
  if (status === 'passed') return labels.qualificationCheckPassed
  if (status === 'failed') return labels.qualificationCheckFailed
  if (status === 'cancelled') return labels.qualificationCheckCancelled
  return labels.qualificationCheckSkipped
}

const checkTone = (status: string) => {
  if (status === 'passed') return 'border-emerald-500/20 bg-emerald-500/10 text-emerald-200'
  if (status === 'failed' || status === 'cancelled') return 'border-red-500/20 bg-red-500/10 text-red-200'
  return 'border-slate-700 bg-slate-900 text-slate-400'
}

export function EngineQualificationPanel({
  engine,
  models,
  selectedModelId,
  qualifying,
  busy,
  lang,
  labels,
  onModelChange,
  onQualify,
  onCancel,
}: Props) {
  const eligibleModels = models.filter(eligibleModel)
  const report = engine.capabilities?.qualification
  const status = normalizeEngineQualificationStatus(engine.capabilities)
  const hasReport = Boolean(report && (
    status !== 'unqualified' || report.checks.length > 0 || report.completedAt
  ))
  const completed = report?.completedAt
    ? new Date(report.completedAt * 1000).toLocaleString(lang)
    : labels.never
  const fingerprint = report?.executableFingerprint
    ? `…${report.executableFingerprint.slice(-16)}`
    : '--'

  return (
    <InsetSurface className="space-y-4 p-4" data-testid="engine-qualification-panel">
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <FlaskConical className="h-4 w-4 text-violet-300" />
            <p className="text-sm font-medium text-slate-100">{labels.qualification}</p>
          </div>
          <p className="mt-1 text-xs leading-5 text-slate-500">{labels.qualificationDescription}</p>
        </div>
        <Badge tone={statusTone(status)}>{statusLabel(status, labels)}</Badge>
      </div>

      <div className="space-y-2">
        <label className="block text-xs font-medium text-slate-400" htmlFor="qualification-model">
          {labels.qualificationModel}
        </label>
        <SelectInput
          id="qualification-model"
          value={selectedModelId}
          onChange={event => onModelChange(event.target.value)}
          disabled={busy || eligibleModels.length === 0}
          className="w-full"
        >
          <option value="">{labels.qualificationModelPlaceholder}</option>
          {eligibleModels.map(model => (
            <option key={model.id} value={model.id}>
              {model.name} · {formatBytes(model.size)}
            </option>
          ))}
        </SelectInput>
        {eligibleModels.length === 0 && (
          <p className="text-xs leading-5 text-amber-300">{labels.qualificationNoModel}</p>
        )}
      </div>

      {qualifying ? (
        <Button
          variant="danger"
          className="w-full"
          icon={<Ban className="h-4 w-4" />}
          onClick={onCancel}
          data-testid="cancel-engine-qualification"
        >
          {labels.qualificationCancel}
        </Button>
      ) : (
        <Button
          variant="violet"
          className="w-full"
          icon={<ShieldCheck className="h-4 w-4" />}
          onClick={onQualify}
          disabled={!selectedModelId || busy}
          data-testid="run-engine-qualification"
        >
          {labels.qualificationRun}
        </Button>
      )}

      {qualifying && (
        <div className="flex items-center gap-2 text-xs text-blue-300" role="status">
          <LoaderCircle className="h-4 w-4 animate-spin" />
          <span>{labels.qualificationRunning}</span>
        </div>
      )}

      <p className="rounded-lg border border-slate-800 bg-slate-950/60 px-3 py-2 text-xs leading-5 text-slate-500">
        {labels.qualificationSafety}
      </p>

      {!report || !hasReport ? (
        <p className="text-xs leading-5 text-amber-300" data-testid="qualification-no-report">
          {labels.qualificationNoReport}
        </p>
      ) : (
        <div className="space-y-3" data-testid="qualification-report">
          <p className="text-xs font-medium uppercase tracking-[0.12em] text-slate-500">
            {labels.qualificationReport}
          </p>
          <div className="space-y-2 text-xs">
            {[
              [labels.qualificationProfile, `${report.schemaVersion} / ${report.profileVersion}`],
              [labels.qualificationExecutionProfile, report.executionProfile
                ? `${report.executionProfile}${report.backend ? ` · ${report.backend}` : ''}`
                : '--'],
              [labels.qualificationFingerprint, fingerprint],
              [labels.qualificationModelEvidence, `${report.modelName || '--'} · ${formatBytes(report.modelSize)}`],
              [labels.qualificationCompleted, completed],
            ].map(([label, value]) => (
              <div key={label} className="flex items-start justify-between gap-3">
                <span className="text-slate-500">{label}</span>
                <span className="min-w-0 text-right text-slate-300" title={String(value)}>{value}</span>
              </div>
            ))}
          </div>
          <div className="space-y-2">
            {report.checks.map(check => (
              <div key={check.name} className={`rounded-lg border px-3 py-2 text-xs ${checkTone(check.status)}`}>
                <div className="flex items-center justify-between gap-3">
                  <span className="font-medium">{checkLabel(check.name, labels)}</span>
                  <span>{checkStatusLabel(check.status, labels)} · {formatMs(check.durationMs)}</span>
                </div>
                {check.detail && <p className="mt-1 break-words opacity-80">{check.detail}</p>}
              </div>
            ))}
          </div>
          {report.diagnostic && (
            <div className="rounded-lg border border-amber-500/20 bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-200">
              <p className="font-medium">{labels.qualificationDiagnostic}</p>
              <p className="mt-1 break-words opacity-80">{report.diagnostic}</p>
            </div>
          )}
        </div>
      )}
    </InsetSurface>
  )
}
