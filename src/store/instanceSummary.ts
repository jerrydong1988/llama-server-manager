import { pathBasename } from '../utils/path'
import type { Instance } from './types'

export function synchronizeInstanceSummary(instance: Instance): Instance {
  const name = instance.config.name || instance.name
  const model = pathBasename(instance.config.model_path)
  const port = instance.config.port

  if (instance.name === name && instance.model === model && instance.port === port) {
    return instance
  }

  return { ...instance, name, model, port }
}
