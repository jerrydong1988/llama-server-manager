import { AlertTriangle, RotateCcw, SlidersHorizontal, Terminal } from 'lucide-react'
import type { EngineInfo, InstanceConfig } from '../../store'
import type { getConfigPageLabels } from '../../i18n/configPageCopy'
import { getEngineCompatibilityMode } from '../../engineCapabilities'
import { Badge, Button, InsetSurface, PathText, Surface } from '../ui'
import { Num } from './shared'

type Labels = ReturnType<typeof getConfigPageLabels>

type Props = {
  config: InstanceConfig
  engine: EngineInfo | null
  labels: Labels
  overrideKeys: Array<keyof InstanceConfig>
  inheritKeys: Array<keyof InstanceConfig>
  set: <K extends keyof InstanceConfig>(key: K, value: InstanceConfig[K]) => void
  inherit: (keys: Array<keyof InstanceConfig>) => void
}

export function LaunchModePanel({ config, engine, labels, overrideKeys, inheritKeys, set, inherit }: Props) {
  const manualMode = config.launch_mode === 'manual'
  const manualRecommended = Boolean(engine && getEngineCompatibilityMode(engine.capabilities) !== 'full')
  return (
    <Surface className="p-4">
      <div className="console-heading-row">
        <h3 className="text-sm font-semibold text-ui-ink">{labels.launchPolicy}</h3>
        <div className="flex flex-wrap gap-1" role="group" aria-label={labels.launchPolicy}>
          {([
            ['managed', labels.managedMode, labels.managedModeDesc, SlidersHorizontal],
            ['manual', labels.manualMode, labels.manualModeDesc, Terminal],
          ] as const).map(([mode, title, description, Icon]) => (
            <button key={mode} type="button" className="console-filter" aria-pressed={config.launch_mode === mode}
              title={description} onClick={() => set('launch_mode', mode)}>
              <Icon size={14} />{title}{mode === 'manual' && manualRecommended && <Badge tone="amber">{labels.manualRecommended}</Badge>}
            </button>
          ))}
        </div>
      </div>
      {manualMode ? (
        <div className="mt-5 space-y-4">
          <InsetSurface className="p-4">
            <p className="text-xs font-medium uppercase tracking-[0.14em] text-ui-muted">{labels.manualEngine}</p>
            <PathText value={engine?.exe || '--'} maxLength={120} className="mt-2 text-ui-body" />
          </InsetSurface>
          <div>
            <label htmlFor="manual-launch-command" className="mb-2 block text-sm font-medium text-ui-ink">{labels.manualCommand}</label>
            <textarea
              id="manual-launch-command"
              value={config.manual_command}
              onChange={event => set('manual_command', event.target.value)}
              placeholder={labels.manualCommandPlaceholder}
              spellCheck={false}
              className="min-h-40 w-full resize-y rounded-xl border border-ui-control bg-ui-surface px-4 py-3 font-mono text-sm leading-6 text-ui-ink outline-none transition focus:border-ui-focus focus:ring-2 focus:ring-ui-focus"
            />
            <p className="mt-2 text-sm leading-6 text-ui-muted">{labels.manualCommandHint}</p>
          </div>
          <Num label={labels.manualPort} value={config.port} onChange={value => set('port', value)} min={1} max={65535} title={labels.manualPortHint} fieldKey="port" />
          <div className="flex items-start gap-3 rounded-lg border border-ui-line bg-ui-warn-bg px-4 py-3 text-sm text-ui-warn">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{labels.manualModeWarning}</span>
          </div>
        </div>
      ) : (
        <div className="mt-2 flex flex-wrap items-center gap-2 border-t border-ui-line pt-2 text-xs text-ui-muted">
          <span title={labels.explicitOverridesDesc}>{labels.explicitOverrides} · {overrideKeys.length}</span>
          {overrideKeys.length > 0 && <Button onClick={() => inherit(inheritKeys)} variant="subtle" size="sm" icon={<RotateCcw size={13} />}>{labels.inheritAll}</Button>}
          <details className="min-w-0"><summary className="cursor-pointer">{labels.modeHelp}</summary>
            <p className="mt-1">{labels.managedModeDesc} {labels.explicitOverridesDesc} {labels.systemArgsHint}</p>
          </details>
        </div>
      )}
    </Surface>
  )
}
