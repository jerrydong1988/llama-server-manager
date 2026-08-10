import type { EngineInfo, InstanceConfig, ModelInfo } from './store/types'
import type { Translations } from './i18n'
import { defaultInstanceConfig } from './store/defaults'
import { assessProjectorMatch } from './modelProjector'
import { PARAMETER_CATALOG } from './parameterCatalog'
import {
  ngramCacheEnabled,
  projectorEnabled,
  reasoningBudgetEnabled,
  speculativeEnabled,
  speculativeType,
} from './configSemantics'

type WarningKey = Extract<keyof Translations['configPage'], `warn${string}`>

export interface Warning {
  field: keyof InstanceConfig
  severity: 'high' | 'medium' | 'low'
  key: WarningKey
}

// Flags represented by managed configuration fields. Custom arguments are
// appended last, so repeating one of these flags can silently override the UI.
export const KNOWN_FLAGS = new Set([
  // Basic
  '-m', '--model', '-a', '--alias', '--lora', '--lora-init-without-apply', '--lora-scaled',
  '-mm', '--mmproj', '-mmu', '--mmproj-url', '--mmproj-auto', '--no-mmproj', '--no-mmproj-auto',
  '--mmproj-offload', '--no-mmproj-offload',
  '--chat-template', '--chat-template-file', '--skip-chat-parsing',
  '--reasoning-format', '-rea', '--reasoning', '--reasoning-budget', '--reasoning-budget-message',
  '--reasoning-preserve', '--no-reasoning-preserve', '--chat-template-kwargs',
  '--jinja', '--no-jinja', '--grammar-file', '--grammar',
  // Performance and context
  '-c', '--ctx-size', '-ngl', '--n-gpu-layers', '-t', '--threads', '-b', '--batch-size',
  '-ub', '--ubatch-size', '-np', '--parallel', '-cb', '--cont-batching', '--no-cont-batching',
  '--cache-prompt', '--no-cache-prompt', '--threads-batch', '--threads-http', '--keep',
  '--cache-reuse', '-cram', '--cache-ram', '--warmup', '--no-warmup',
  '-ctxcp', '--ctx-checkpoints', '-cms', '--checkpoint-min-step', '--swa-full',
  // RoPE / YaRN
  '--rope-scaling', '--rope-scale', '--rope-freq-base', '--rope-freq-scale',
  '--yarn-ext-factor', '--yarn-attn-factor', '--yarn-beta-slow', '--yarn-beta-fast', '--yarn-orig-ctx',
  // Flash attention and memory
  '-fa', '--flash-attn', '--n-cpu-moe', '--cpu-moe', '-cmoe', '--cpu-moe-layers',
  '--load-mode', '-lm', '--mlock', '--mmap', '--no-mmap', '--repack', '--no-repack',
  '--numa', '--check-tensors', '--perf', '--no-perf', '--fit', '-fitt', '--fit-target',
  '-fitc', '--fit-ctx', '--direct-io', '-dio',
  // KV cache
  '-ctk', '--cache-type-k', '-ctv', '--cache-type-v',
  '-ctkd', '--spec-draft-type-k', '--cache-type-k-draft',
  '-ctvd', '--spec-draft-type-v', '--cache-type-v-draft',
  '--kv-unified', '--no-kv-unified', '--kv-offload', '--no-kv-offload',
  '--cache-idle-slots', '--no-cache-idle-slots',
  // GPU and device
  '-dev', '--device', '-sm', '--split-mode', '-ts', '--tensor-split', '-mg', '--main-gpu', '--override-kv',
  // Server and network
  '--host', '--port', '--api-key', '--api-key-file', '--ssl-key-file', '--ssl-cert-file',
  '--path', '--api-prefix', '--cors-origins', '--cors-methods', '--cors-headers',
  '--cors-credentials', '--no-cors-credentials', '--ui', '--webui', '--no-ui', '--no-webui', '--offline',
  '--ui-config-file', '--webui-config-file', '--ui-config', '--webui-config',
  '--ui-mcp-proxy', '--webui-mcp-proxy', '--no-ui-mcp-proxy', '--no-webui-mcp-proxy',
  '--tools-runtime', '--mcp-servers-config', '--mcp-servers-json',
  '-ag', '--agent', '-no-ag', '--no-agent',
  // Embedding and server features
  '--embedding', '--embeddings', '--pooling', '--embd-normalize', '--rerank', '--reranking',
  '--metrics', '--props', '--slots', '--no-slots', '--slot-save-path', '--log-prompts-dir', '-sps',
  '--slot-prompt-similarity', '--context-shift', '--no-context-shift',
  '--prefill-assistant', '--no-prefill-assistant', '--rpc', '--sse-ping-interval', '--reuse-port',
  // Multi-model and media
  '--models-dir', '--models-preset', '--models-max', '--models-autoload', '--no-models-autoload',
  '--image-min-tokens', '--image-max-tokens', '--mtmd-batch-max-tokens', '--tags', '--media-path', '--tools',
  // Generation and sampling
  '-n', '--n-predict', '--ignore-eos', '--json-schema', '-jf', '--json-schema-file',
  '--temp', '--temperature', '--top-k', '--top-p', '--repeat-penalty', '--seed', '--min-p',
  '--presence-penalty', '--frequency-penalty', '--repeat-last-n', '-r', '--reverse-prompt',
  '-sp', '--special', '--spm-infill', '-bs', '--backend-sampling',
  '--mirostat', '--mirostat-lr', '--mirostat-ent', '--xtc-probability', '--xtc-threshold',
  '--dynatemp-range', '--dynatemp-exp', '--typical', '--typical-p', '--dry-multiplier',
  '--dry-base', '--dry-allowed-length', '--dry-penalty-last-n', '--dry-sequence-breaker',
  '--adaptive-target', '--adaptive-decay', '--top-nsigma', '--top-n-sigma', '-l', '--logit-bias',
  '--samplers', '--sampler-seq', '--sampling-seq',
  // Speculative decoding
  '--spec-type', '-md', '--model-draft', '-ngld', '--spec-draft-ngl', '--n-gpu-layers-draft',
  '--spec-draft-n-max', '--spec-draft-n-min', '--spec-draft-p-min', '--spec-draft-p-split',
  '--spec-draft-device', '-lcs', '--lookup-cache-static', '-lcd', '--lookup-cache-dynamic',
  '--spec-default', '--spec-draft-backend-sampling', '--no-spec-draft-backend-sampling',
  '-td', '--spec-draft-threads', '-tbd', '--spec-draft-threads-batch',
  // Miscellaneous managed fields
  '-to', '--timeout', '--sleep-idle-seconds', '-v', '--verbose', '--log-verbose',
])

for (const definition of Object.values(PARAMETER_CATALOG)) {
  for (const flag of definition?.flags ?? []) KNOWN_FLAGS.add(flag)
}

function hasMetadata(model: ModelInfo | null | undefined): boolean {
  return Boolean(model?.capabilities?.metadata_complete)
}

function hasBuiltinMtp(model: ModelInfo | null | undefined): boolean {
  return Boolean(model?.capabilities?.has_builtin_mtp ?? model?.has_mtp_head)
}

function isVisionModel(model: ModelInfo | null | undefined): boolean | null {
  if (!model) return null
  if (model.capabilities?.vision_status === 'confirmed') return true
  if (model.capabilities?.vision_status === 'text-only') return false
  if (model.capabilities?.is_vision_model) return true
  return null
}

function isMmprojArtifact(model: ModelInfo | null | undefined): boolean {
  return Boolean(model?.capabilities?.is_mmproj || model?.file_type === 'mmproj')
}

function isLoopbackHost(host: string): boolean {
  const normalized = host.trim().toLowerCase().replace(/^\[(.*)\]$/, '$1')
  return normalized === 'localhost' || normalized === '::1' || /^127(?:\.\d{1,3}){3}$/.test(normalized)
}

function hasWildcardCorsOrigin(origins: string): boolean {
  return origins.split(',').some(origin => origin.trim() === '*')
}

function isValidJson(value: string): boolean {
  if (!value.trim()) return true
  try {
    JSON.parse(value)
    return true
  } catch {
    return false
  }
}

function isValidToolsRuntime(value: string): boolean {
  const normalized = value.trim()
  if (!normalized) return true
  return /^(?:docker|podman|docker-container|podman-container|ssh):\S+$/.test(normalized)
}

type CustomArgToken = { value: string; quoted: boolean }

function tokenizeCustomArgRows(rows: readonly string[]): CustomArgToken[] {
  const result: CustomArgToken[] = []
  for (const row of rows) {
    let current = ''
    let inQuotes = false
    let tokenStarted = false
    let tokenQuoted = false
    for (let index = 0; index < row.length; index += 1) {
      const character = row[index]
      if (character === '"') {
        inQuotes = !inQuotes
        tokenStarted = true
        tokenQuoted = true
        continue
      }
      if (character === '\\') {
        tokenStarted = true
        let count = 1
        while (row[index + 1] === '\\') {
          index += 1
          count += 1
        }
        if (row[index + 1] === '"') {
          current += '\\'.repeat(Math.floor(count / 2))
          index += 1
          if (count % 2 === 0) inQuotes = !inQuotes
          else current += '"'
        } else {
          current += '\\'.repeat(count)
        }
        continue
      }
      if (/\s/.test(character) && !inQuotes) {
        if (tokenStarted) {
          result.push({ value: current, quoted: tokenQuoted })
          current = ''
          tokenStarted = false
          tokenQuoted = false
        }
        continue
      }
      current += character
      tokenStarted = true
    }
    if (tokenStarted) result.push({ value: current, quoted: tokenQuoted })
  }
  return result
}

/** Tokenize custom-argument rows with the same quote/backslash rules as server.rs. */
export function tokenizeCustomArgs(rows: readonly string[]): string[] {
  return tokenizeCustomArgRows(rows).map(token => token.value)
}

function customFlags(config: InstanceConfig): Set<string> {
  const tokens = tokenizeCustomArgRows(config.custom_args)
  const flags = new Set<string>()
  tokens.forEach((token, index) => {
    if (!token.value.startsWith('-')) return
    const previous = tokens[index - 1]
    const isQuotedValue = token.quoted
      && Boolean(previous && !previous.quoted && previous.value.startsWith('-') && !previous.value.includes('='))
    if (!isQuotedValue) flags.add(token.value.split('=', 1)[0])
  })
  return flags
}

function samplerNames(config: InstanceConfig): Set<string> | null {
  const aliases: Record<string, string> = {
    nucleus: 'top_p', temp: 'temperature', typ: 'typ_p', typical: 'typ_p',
  }
  if (config.sampler_seq.trim()) {
    const chars: Record<string, string> = {
      d: 'dry', k: 'top_k', y: 'typ_p', p: 'top_p', s: 'top_n_sigma', m: 'min_p',
      t: 'temperature', x: 'xtc', i: 'infill', e: 'penalties', a: 'adaptive_p',
    }
    return new Set([...config.sampler_seq.trim()].map(character => chars[character]).filter(Boolean))
  }
  if (!config.samplers.trim()) return null
  return new Set(
    config.samplers
      .split(/[;,]/)
      .map(value => value.trim().toLowerCase())
      .filter(Boolean)
      .map(value => aliases[value] ?? value),
  )
}

function customizedSamplerMissing(config: InstanceConfig, defaults: InstanceConfig): boolean {
  const names = samplerNames(config)
  if (!names || config.mirostat > 0) return false
  const checks: Array<[boolean, string]> = [
    [Math.abs(config.temp - defaults.temp) > 0.001 || config.dynatemp_range > 0, 'temperature'],
    [config.top_k !== defaults.top_k, 'top_k'],
    [Math.abs(config.top_p - defaults.top_p) > 0.001, 'top_p'],
    [Math.abs(config.min_p - defaults.min_p) > 0.001, 'min_p'],
    [Math.abs(config.typical_p - defaults.typical_p) > 0.001, 'typ_p'],
    [config.top_n_sigma >= 0, 'top_n_sigma'],
    [config.xtc_probability > 0, 'xtc'],
    [config.dry_multiplier > 0, 'dry'],
    [config.adaptive_target >= 0, 'adaptive_p'],
    [config.repeat_penalty !== defaults.repeat_penalty
      || config.presence_penalty !== defaults.presence_penalty
      || config.frequency_penalty !== defaults.frequency_penalty
      || config.repeat_last_n !== defaults.repeat_last_n, 'penalties'],
  ]
  return checks.some(([customized, sampler]) => customized && !names.has(sampler))
}

function hasMirostatIgnoredSettings(config: InstanceConfig, defaults: InstanceConfig): boolean {
  return config.top_k !== defaults.top_k
    || Math.abs(config.top_p - defaults.top_p) > 0.001
    || Math.abs(config.min_p - defaults.min_p) > 0.001
    || Math.abs(config.typical_p - defaults.typical_p) > 0.001
    || config.top_n_sigma >= 0
    || config.xtc_probability > 0
    || config.dynatemp_range > 0
    || config.dry_multiplier > 0
    || config.adaptive_target >= 0
    || config.repeat_penalty !== defaults.repeat_penalty
    || config.presence_penalty !== defaults.presence_penalty
    || config.frequency_penalty !== defaults.frequency_penalty
    || config.repeat_last_n !== defaults.repeat_last_n
    || Boolean(config.samplers.trim())
    || Boolean(config.sampler_seq.trim())
}

export function validateConfig(
  config: InstanceConfig,
  model: ModelInfo | null | undefined,
  _engine: EngineInfo | null | undefined,
  projector?: ModelInfo | null,
): Warning[] {
  const warnings: Warning[] = []
  const defaults = defaultInstanceConfig()
  const flags = customFlags(config)
  const specType = speculativeType(config)
  const specActive = speculativeEnabled(config)
  const projectorActive = projectorEnabled(config)
  const hasGrammar = Boolean(config.grammar || config.grammar_file || config.json_schema || config.json_schema_file)
  const hasReasoningBudget = reasoningBudgetEnabled(config)

  // Reasoning parsing/preservation remain meaningful independently. Only the
  // actual token budget and its forced message become inactive when thinking is off.
  if (config.reasoning === 'off' && (hasReasoningBudget || config.reasoning_budget_message.trim())) {
    warnings.push({ field: 'reasoning_budget', severity: 'low', key: 'warnA1' })
  }

  // External draft requirements, including sources supplied through the managed escape hatch.
  if (specActive) {
    const isDraftMtp = specType.includes('draft-mtp')
    const needsExternalDraft = ['draft-simple', 'draft-eagle3', 'draft-dflash', 'draft-dspark']
      .some(type => specType.includes(type))
    const hasExternalDraft = Boolean(config.draft_model_path.trim())
      || flags.has('--spec-draft-hf')
      || flags.has('--model-draft')
      || flags.has('-md')
    if (needsExternalDraft && !hasExternalDraft) {
      warnings.push({ field: 'draft_model_path', severity: 'medium', key: 'warnA3' })
    } else if (isDraftMtp && !hasExternalDraft && !hasBuiltinMtp(model)) {
      warnings.push({
        field: 'draft_model_path',
        severity: hasMetadata(model) ? 'medium' : 'low',
        key: hasMetadata(model) ? 'warnA3MtpNeedsDraft' : 'warnA3MtpUnknown',
      })
    }
  }

  // Disabled speculative decoding leaves these settings inert in the generated command.
  if (!specActive && (
    Boolean(config.draft_model_path.trim())
    || config.draft_gpu_layers !== defaults.draft_gpu_layers
    || (config.draft_tokens !== defaults.draft_tokens && config.draft_tokens !== 0)
    || config.spec_draft_n_min !== defaults.spec_draft_n_min
    || config.spec_draft_p_min !== defaults.spec_draft_p_min
    || Math.abs(config.spec_draft_p_split - defaults.spec_draft_p_split) > 0.001
    || Boolean(config.spec_draft_device.trim())
    || Boolean(config.lookup_cache_static.trim() || config.lookup_cache_dynamic.trim())
    || config.spec_default
    || !config.spec_draft_backend_sampling
    || config.spec_draft_threads > 0
    || config.spec_draft_threads_batch > 0
    || Boolean(config.cache_type_draft_k.trim() || config.cache_type_draft_v.trim())
  )) {
    warnings.push({ field: 'spec_type', severity: 'low', key: 'warnA5' })
  }

  // llama-server allocates the configured total context across parallel slots.
  if (!config.ctx_size_auto && config.ctx_size > 0 && model?.context_length) {
    const parallel = config.parallel > 0 ? config.parallel : 1
    const perSlotContext = Math.floor(config.ctx_size / parallel)
    if (perSlotContext > model.context_length) {
      warnings.push({ field: 'ctx_size', severity: 'medium', key: 'warnA7' })
    }
  }

  if ((config.image_min_tokens > 0 || config.image_max_tokens > 0) && !projectorActive) {
    warnings.push({ field: 'image_min_tokens', severity: 'medium', key: 'warnA8' })
  }

  if (config.swa_full && model?.capabilities?.has_swa === false) {
    warnings.push({ field: 'swa_full', severity: 'low', key: 'warnA9' })
  }

  if (config.grammar && config.grammar_file) {
    warnings.push({ field: 'grammar', severity: 'low', key: 'warnA11' })
  }
  if (config.chat_template && config.chat_template_file) {
    warnings.push({ field: 'chat_template', severity: 'low', key: 'warnA12' })
  }
  if (Boolean(config.ssl_key_file) !== Boolean(config.ssl_cert_file)) {
    warnings.push({
      field: config.ssl_key_file ? 'ssl_cert_file' : 'ssl_key_file',
      severity: 'high',
      key: 'warnA14',
    })
  }

  if (config.cache_reuse > 0 && !config.cache_prompt) {
    warnings.push({ field: 'cache_prompt', severity: 'low', key: 'warnB2' })
  }
  if (!config.slots_enabled && config.slot_save_path.trim()) {
    warnings.push({ field: 'slots_enabled', severity: 'low', key: 'warnB3' })
  }
  if (config.mirostat > 0 && hasMirostatIgnoredSettings(config, defaults)) {
    warnings.push({ field: 'mirostat', severity: 'low', key: 'warnB4' })
  }
  if (customizedSamplerMissing(config, defaults)) {
    warnings.push({ field: config.sampler_seq ? 'sampler_seq' : 'samplers', severity: 'low', key: 'warnB5' })
  }
  if (config.pooling.trim() && !config.embedding) {
    warnings.push({ field: 'pooling', severity: 'low', key: 'warnB6' })
  }
  if (config.gpu_layers === 0 && config.device.trim()) {
    warnings.push({ field: 'gpu_layers', severity: 'low', key: 'warnB7' })
  }
  if (specActive && (config.lookup_cache_static || config.lookup_cache_dynamic) && !ngramCacheEnabled(config)) {
    warnings.push({ field: 'lookup_cache_static', severity: 'low', key: 'warnB9' })
  }
  if (config.json_schema && config.json_schema_file) {
    warnings.push({ field: 'json_schema', severity: 'low', key: 'warnB11' })
  }

  // Backend sampling is experimental on every compute backend. Report actual
  // feature conflicts rather than inferring incompatibility from ROCm or CPU.
  if (config.backend_sampling) {
    warnings.push({ field: 'backend_sampling', severity: 'low', key: 'warnBackendSamplingExperimental' })
    if (hasGrammar) {
      warnings.push({ field: 'backend_sampling', severity: 'medium', key: 'warnBackendSamplingGrammar' })
    }
    if (hasReasoningBudget) {
      warnings.push({ field: 'backend_sampling', severity: 'medium', key: 'warnBackendSamplingReasoning' })
    }
    if (specActive) {
      warnings.push({ field: 'backend_sampling', severity: 'medium', key: 'warnBackendSamplingSpeculative' })
    }
  }

  if (config.cache_idle_slots && config.cache_ram === 0) {
    warnings.push({ field: 'cache_idle_slots', severity: 'medium', key: 'warnCacheIdleWithoutRam' })
  }
  if (projectorActive && config.context_shift) {
    warnings.push({ field: 'context_shift', severity: 'medium', key: 'warnMultimodalContextShift' })
  }
  if (projectorActive && config.cache_reuse > 0) {
    warnings.push({ field: 'cache_reuse', severity: 'medium', key: 'warnMultimodalCacheReuse' })
  }
  const mcpServersEnabled = Boolean(config.mcp_servers_config.trim() || config.mcp_servers_json.trim())
  const privilegedToolsEnabled = Boolean(config.tools.trim() || config.agent || mcpServersEnabled || config.ui_mcp_proxy)
  if ((config.tools.trim() || config.agent || mcpServersEnabled) && !config.jinja) {
    warnings.push({ field: 'jinja', severity: 'high', key: 'warnToolsRequireJinja' })
  }
  if (!isValidJson(config.mcp_servers_json)) {
    warnings.push({ field: 'mcp_servers_json', severity: 'high', key: 'warnMcpServersJsonInvalid' })
  }
  if (!isValidToolsRuntime(config.tools_runtime)) {
    warnings.push({ field: 'tools_runtime', severity: 'high', key: 'warnToolsRuntimeInvalid' })
  }
  if (privilegedToolsEnabled && (!isLoopbackHost(config.host) || hasWildcardCorsOrigin(config.cors_origins))) {
    warnings.push({ field: 'host', severity: 'high', key: 'warnPrivilegedToolsExposure' })
  }
  if (config.samplers.trim() && config.sampler_seq.trim()) {
    warnings.push({ field: 'sampler_seq', severity: 'low', key: 'warnSamplerDefinitionsConflict' })
  }

  // Model/projector compatibility based on positive capability and source evidence.
  if (isMmprojArtifact(model)) {
    warnings.push({ field: 'model_path', severity: 'medium', key: 'warnC5' })
  }
  if (projectorActive && config.mmproj_path.trim()) {
    const vision = isVisionModel(model)
    if (vision === false) {
      warnings.push({ field: 'mmproj_path', severity: 'low', key: 'warnC1' })
    } else if (model) {
      const match = assessProjectorMatch(model, projector)
      if (match.confidence === 'mismatch') {
        warnings.push({ field: 'mmproj_path', severity: 'medium', key: 'warnC1Mismatch' })
      } else if (match.confidence === 'weak') {
        warnings.push({ field: 'mmproj_path', severity: 'low', key: 'warnC1Weak' })
      } else if (match.confidence === 'unknown') {
        warnings.push({ field: 'mmproj_path', severity: 'low', key: 'warnC1Unknown' })
      }
    } else {
      warnings.push({ field: 'mmproj_path', severity: 'low', key: 'warnC1Unknown' })
    }
  }

  if (config.n_predict === -1 && config.ignore_eos) {
    warnings.push({ field: 'n_predict', severity: 'medium', key: 'warnC3' })
  }
  if (config.draft_model_path && specType.includes('draft-mtp') && hasBuiltinMtp(model)) {
    warnings.push({ field: 'draft_model_path', severity: 'low', key: 'warnC4' })
  }

  if ([...flags].some(flag => KNOWN_FLAGS.has(flag))) {
    warnings.push({ field: 'custom_args', severity: 'medium', key: 'warnD1' })
  }

  return warnings
}
