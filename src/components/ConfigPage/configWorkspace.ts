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

const REVIEW_FIELD_ALIASES: Partial<Record<keyof InstanceConfig, Array<keyof InstanceConfig>>> = {
  gpu_layers: ['gpu_layers', 'gpu_layers_auto'],
  ctx_size: ['ctx_size', 'ctx_size_auto'],
  mmproj_mode: ['mmproj_mode', 'mmproj_auto', 'no_mmproj'],
  numa_mode: ['numa_mode', 'numa'],
  fit_mode: ['fit_mode', 'fit'],
  kv_unified_mode: ['kv_unified_mode', 'kv_unified'],
}

const REVIEW_FIELD_CANONICAL = new Map<keyof InstanceConfig, keyof InstanceConfig>(
  Object.entries(REVIEW_FIELD_ALIASES).flatMap(([canonical, keys]) =>
    (keys ?? []).map(key => [key, canonical as keyof InstanceConfig]),
  ),
)

export const canonicalConfigField = (key: keyof InstanceConfig): keyof InstanceConfig =>
  REVIEW_FIELD_CANONICAL.get(key) ?? key

export const canonicalConfigFields = (keys: Iterable<keyof InstanceConfig>): Array<keyof InstanceConfig> =>
  [...new Set([...keys].map(canonicalConfigField))]

export const reviewFieldKeys = (key: keyof InstanceConfig): Array<keyof InstanceConfig> =>
  REVIEW_FIELD_ALIASES[canonicalConfigField(key)] ?? [key]

export const restoreReviewField = (config: InstanceConfig, baseline: InstanceConfig, key: keyof InstanceConfig): InstanceConfig => {
  const next = { ...config }
  const currentOverrides = new Set(config.explicit_overrides ?? [])
  const savedOverrides = new Set(baseline.explicit_overrides ?? [])
  for (const field of reviewFieldKeys(key)) {
    next[field] = baseline[field] as never
    if (savedOverrides.has(field)) currentOverrides.add(field)
    else currentOverrides.delete(field)
  }
  next.explicit_overrides = [...currentOverrides]
  return next
}

const reviewFieldValue = (config: InstanceConfig, key: keyof InstanceConfig): unknown => {
  switch (canonicalConfigField(key)) {
    case 'mmproj_mode':
      return config.mmproj_mode || (config.no_mmproj ? 'off' : config.mmproj_auto ? 'on' : '')
    case 'numa_mode':
      return config.numa_mode || (config.numa ? 'distribute' : '')
    case 'fit_mode':
      return config.fit_mode || (config.fit ? 'on' : '')
    case 'kv_unified_mode':
      return config.kv_unified_mode || (config.kv_unified ? 'on' : '')
    default:
      return config[key]
  }
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
    numa_mode: t.configPage.numa,
    fit_mode: t.configPage.fit,
    kv_unified_mode: t.configPage.kvUnified,
    mmproj_mode: t.configPage.mmprojAuto,
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
  canonicalConfigFields(Object.keys(local) as Array<keyof InstanceConfig>)
    .filter(key => key !== 'explicit_overrides')
    .filter(key => !isEqualValue(reviewFieldValue(local, key), reviewFieldValue(baseline, key)))
    .map(key => ({
      key,
      label: fieldLabel(key, t),
      before: formatConfigValue(key, reviewFieldValue(baseline, key), labels, t),
      after: formatConfigValue(key, reviewFieldValue(local, key), labels, t),
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
