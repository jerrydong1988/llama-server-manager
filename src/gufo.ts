import { defaultInstanceConfig } from './store/defaults'
import type { EngineInfo, InstanceConfig } from './store/types'

/** Engine changes never reinterpret one engine's tuning as another engine's flags. */
export function configForEngine(config: InstanceConfig, engine: EngineInfo): InstanceConfig {
  const kind = engine.kind ?? 'llama_cpp'
  if ((config.engine_kind ?? 'llama_cpp') === kind) return { ...config, engine_id: engine.id }
  const next = { ...defaultInstanceConfig(), id: config.id, name: config.name,
    model_path: config.model_path, mmproj_path: config.mmproj_path, alias: config.alias,
    port: config.port, engine_id: engine.id, engine_kind: kind }
  if (kind === 'gufo') {
    next.ctx_size = 32768
    next.parallel = 1
    next.temp = 1
    next.top_k = 20
    next.min_p = 0
    if (/27b/i.test(config.model_path)) next.gufo.model_family = 'qwen38_27b'
  }
  return next
}
