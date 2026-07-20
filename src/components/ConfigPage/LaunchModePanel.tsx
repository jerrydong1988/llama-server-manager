import { AlertTriangle, RotateCcw, SlidersHorizontal, Terminal } from 'lucide-react'
import type { EngineInfo, InstanceConfig } from '../../store'
import type { getConfigPageLabels } from '../../i18n/configPageCopy'
import { getEngineCompatibilityMode } from '../../engineCapabilities'
import { Badge, Button, InsetSurface, PathText, SectionHeader, Surface } from '../ui'

type Labels = ReturnType<typeof getConfigPageLabels>

type Props = {
  config: InstanceConfig
  engine: EngineInfo | null
  labels: Labels
  overrideKeys: Array<keyof InstanceConfig>
  inheritKeys: Array<keyof InstanceConfig>
  set: (key: keyof InstanceConfig, value: any) => void
  inherit: (keys: Array<keyof InstanceConfig>) => void
}

export function LaunchModePanel({ config, engine, labels, overrideKeys, inheritKeys, set, inherit }: Props) {
  const manualMode = config.launch_mode === 'manual'
  const manualRecommended = Boolean(engine && getEngineCompatibilityMode(engine.capabilities) !== 'full')
  return (
    <Surface className="p-5">
      <SectionHeader title={labels.launchPolicy} />
      <div className="mt-4 grid gap-3 md:grid-cols-2">
        {([
          ['managed', labels.managedMode, labels.managedModeDesc, SlidersHorizontal],
          ['manual', labels.manualMode, labels.manualModeDesc, Terminal],
        ] as const).map(([mode, title, description, Icon]) => {
          const selected = config.launch_mode === mode
          return (
            <button
              key={mode}
              type="button"
              onClick={() => set('launch_mode', mode)}
              className={`rounded-xl border p-4 text-left transition ${selected
                ? 'border-blue-500/50 bg-blue-500/10 ring-1 ring-blue-500/20'
                : 'border-slate-200 bg-slate-50 hover:border-slate-300 dark:border-slate-800 dark:bg-slate-950/40 dark:hover:border-slate-700'}`}
            >
              <div className="flex items-center gap-2">
                <Icon className={`h-4 w-4 ${selected ? 'text-blue-300' : 'text-slate-400'}`} />
                <span className="font-medium text-slate-900 dark:text-slate-100">{title}</span>
                {mode === 'manual' && manualRecommended && <Badge tone="amber">{labels.manualRecommended}</Badge>}
              </div>
              <p className="mt-2 text-sm leading-6 text-slate-500 dark:text-slate-400">{description}</p>
            </button>
          )
        })}
      </div>

      {manualMode ? (
        <div className="mt-5 space-y-4">
          <InsetSurface className="p-4">
            <p className="text-xs font-medium uppercase tracking-[0.14em] text-slate-500">{labels.manualEngine}</p>
            <PathText value={engine?.exe || '--'} maxLength={120} className="mt-2 text-slate-700 dark:text-slate-200" />
          </InsetSurface>
          <div>
            <label className="mb-2 block text-sm font-medium text-slate-800 dark:text-slate-200">{labels.manualCommand}</label>
            <textarea
              value={config.manual_command}
              onChange={event => set('manual_command', event.target.value)}
              placeholder={labels.manualCommandPlaceholder}
              spellCheck={false}
              className="min-h-40 w-full resize-y rounded-xl border border-slate-200 bg-white px-4 py-3 font-mono text-sm leading-6 text-slate-900 outline-none transition focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 dark:border-slate-700 dark:bg-slate-950 dark:text-slate-100"
            />
            <p className="mt-2 text-sm leading-6 text-slate-500">{labels.manualCommandHint}</p>
          </div>
          <div className="flex items-start gap-3 rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-700 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{labels.manualModeWarning}</span>
          </div>
        </div>
      ) : (
        <div className="mt-5 rounded-xl border border-slate-200 bg-slate-50 p-4 dark:border-slate-800 dark:bg-slate-950/40">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.explicitOverrides}</p>
              <p className="mt-1 text-sm text-slate-500">{labels.explicitOverridesDesc}</p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <Badge tone={overrideKeys.length > 0 ? 'blue' : 'emerald'}>{overrideKeys.length} {labels.activeParams}</Badge>
              {overrideKeys.length > 0 && (
                <Button onClick={() => inherit(inheritKeys)} variant="secondary" size="sm" icon={<RotateCcw className="h-4 w-4" />}>
                  {labels.inheritAll}
                </Button>
              )}
            </div>
          </div>
          {overrideKeys.length === 0 && <p className="mt-3 text-sm text-emerald-700 dark:text-emerald-300">{labels.noExplicitOverrides}</p>}
          <p className="mt-3 text-xs text-slate-500">{labels.systemArgsHint}</p>
        </div>
      )}
    </Surface>
  )
}
