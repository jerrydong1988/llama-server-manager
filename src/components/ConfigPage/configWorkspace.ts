import type { InstanceConfig, ModelInfo } from '../../store'
import { isPathWithinRoot, normalizePath, pathJoin } from '../../utils/path'

export interface PickerNode {
  name: string
  path: string
  isDir: boolean
  children?: Map<string, PickerNode>
  model?: ModelInfo
}

export const buildPickerTree = (rootDir: string, models: ModelInfo[]): PickerNode => {
  const normalizedRoot = normalizePath(rootDir)
  const root: PickerNode = { name: rootDir, path: normalizedRoot, isDir: true, children: new Map() }

  for (const model of models) {
    const normalizedPath = normalizePath(model.path)
    if (!isPathWithinRoot(normalizedPath, normalizedRoot)) {
      continue
    }

    const relative = normalizedPath.slice(normalizedRoot.length).replace(/^\/+/, '')
    if (!relative) {
      continue
    }

    const parts = relative.split('/')
    let cursor = root

    for (let index = 0; index < parts.length; index += 1) {
      const part = parts[index]
      if (index === parts.length - 1) {
        cursor.children!.set(part, { name: part, path: model.path, isDir: false, model })
      } else {
        if (!cursor.children!.has(part)) {
          cursor.children!.set(part, {
            name: part,
            path: pathJoin(cursor.path, part),
            isDir: true,
            children: new Map(),
          })
        }
        cursor = cursor.children!.get(part)!
      }
    }
  }

  return root
}

export const countActive = (activeParams: Set<keyof InstanceConfig>, keys: Array<keyof InstanceConfig>) =>
  keys.filter(key => activeParams.has(key)).length

export type ConfigChange = {
  key: keyof InstanceConfig
  label: string
  before: string
  after: string
}

export type TemplateSnapshot = {
  templateId: string
  templateTitle: string
  config: InstanceConfig
}

export type ChangeGroup = {
  id: string
  title: string
  keys: Array<keyof InstanceConfig>
}

export const isEqualValue = (left: unknown, right: unknown) => {
  if (Array.isArray(left) || Array.isArray(right)) {
    return JSON.stringify(left ?? []) === JSON.stringify(right ?? [])
  }
  return left === right
}

export const formatValue = (value: unknown, labels: Record<string, string>) => {
  if (Array.isArray(value)) {
    return value.length > 0 ? value.join(' ') : labels.emptyValue
  }
  if (typeof value === 'boolean') {
    return value ? labels.on : labels.off
  }
  if (value === '' || value === null || value === undefined) {
    return labels.emptyValue
  }
  return String(value)
}

export const formatConfigValue = (key: keyof InstanceConfig, value: unknown, labels: Record<string, string>, t: any) => (
  key === 'custom_args'
    ? `${Array.isArray(value) ? value.length : 0} ${t.configPage.vectorCleanupItems}`
    : formatValue(value, labels)
)

export const fieldLabel = (key: keyof InstanceConfig, t: any) => {
  const labelMap: Partial<Record<keyof InstanceConfig, string>> = {
    model_path: t.configPage.modelPath,
    alias: t.configPage.alias,
    chat_template: t.configPage.chatTemplate,
    host: t.configPage.host,
    port: t.configPage.portLabel,
    gpu_layers: t.configPage.gpuLayers,
    gpu_layers_auto: t.configPage.gpuLayersAuto,
    ctx_size: t.configPage.ctxSize,
    ctx_size_auto: t.configPage.ctxAuto,
    embedding: t.configPage.embedding,
    pooling: t.configPage.pooling,
    reasoning: t.configPage.reasoningSwitch,
    reasoning_format: t.configPage.reasoningFormat,
    reasoning_effort: t.configPage.reasoningEffort,
    reasoning_budget: t.configPage.reasoningBudget,
    reasoning_budget_message: t.configPage.reasoningBudgetMsg,
    draft_model_path: t.configPage.draftModel,
    draft_tokens: t.configPage.draftTokens,
    spec_type: t.configPage.specType,
    spec_draft_n_min: t.configPage.specDraftNMin,
    temp: t.configPage.temp,
    top_k: t.configPage.topK,
    top_p: t.configPage.topP,
    repeat_penalty: t.configPage.repeatPenalty,
    n_predict: t.configPage.nPredict,
    ignore_eos: t.configPage.ignoreEos,
    reverse_prompt: t.configPage.reversePrompt,
    threads: t.configPage.threads,
    threads_batch: t.configPage.threadsBatch,
    batch_size: t.configPage.batchSize,
    ubatch_size: t.configPage.ubatchSize,
    parallel: t.configPage.parallel,
    cont_batching: t.configPage.contBatching,
    flash_attn: t.configPage.flashAttn,
    mlock: t.configPage.mlock,
    no_mmap: t.configPage.noMmap,
    no_repack: t.configPage.noRepack,
    numa: t.configPage.numa,
    cache_ram: t.configPage.cacheRam,
    metrics: t.configPage.metrics,
    props: t.configPage.props,
    perf: t.configPage.perf,
    verbose: t.configPage.verbose,
    custom_args: t.configPage.customArgs,
  }
  return labelMap[key] || String(key).replace(/_/g, ' ')
}

export const getConfigChanges = (local: InstanceConfig, baseline: InstanceConfig, t: any, labels: Record<string, string>): ConfigChange[] =>
  (Object.keys(local) as Array<keyof InstanceConfig>)
    .filter(key => key !== 'explicit_overrides')
    .filter(key => !isEqualValue(local[key], baseline[key]))
    .map(key => ({
      key,
      label: fieldLabel(key, t),
      before: formatConfigValue(key, baseline[key], labels, t),
      after: formatConfigValue(key, local[key], labels, t),
    }))

export const getTemplateChanges = (local: InstanceConfig, changes: Partial<InstanceConfig>, t: any, labels: Record<string, string>): ConfigChange[] =>
  (Object.keys(changes) as Array<keyof InstanceConfig>)
    .filter(key => !isEqualValue(local[key], changes[key]))
    .map(key => ({
      key,
      label: fieldLabel(key, t),
      before: formatConfigValue(key, local[key], labels, t),
      after: formatConfigValue(key, changes[key], labels, t),
    }))

export const groupTemplateChanges = (changes: ConfigChange[], groups: ChangeGroup[], otherTitle: string) => {
  const grouped = groups
    .map(group => ({
      ...group,
      changes: changes.filter(change => group.keys.includes(change.key)),
    }))
    .filter(group => group.changes.length > 0)

  const groupedKeys = new Set(grouped.flatMap(group => group.changes.map(change => change.key)))
  const otherChanges = changes.filter(change => !groupedKeys.has(change.key))

  return otherChanges.length > 0
    ? [...grouped, { id: 'other', title: otherTitle, keys: [], changes: otherChanges }]
    : grouped
}
