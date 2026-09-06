import type { EngineCapabilities } from './store/types'

export type ParameterEngine = {
  version?: string
  capabilities?: EngineCapabilities
}

/** Parse official build formats without treating a commit hash or SDK version as a build. */
export function engineBuildNumber(version?: string): number | undefined {
  const value = version?.trim() ?? ''
  const match = value.match(/\bbuild\s+(\d+)\b/i)
    ?? value.match(/^(?:version:\s*)?b(\d+)\b/i)
    ?? value.match(/^(?:version:\s*)?(\d{4,})(?=\s|\(|$)/i)
  return match ? Number(match[1]) : undefined
}

// ggml-org/llama.cpp PR #25532, dd1ea524333b1e697489067d7a4c39c60d32beee (b10355).
// The flag predates this change, so finding it in --help cannot establish this support.
export function speculativeBackendSamplingSupport(engine?: ParameterEngine | null): boolean | undefined {
  const build = engineBuildNumber(engine?.version)
  return build === undefined ? undefined : build >= 10355
}
