export const FALLBACK_SPECULATIVE_TYPES = [
  'none',
  'draft-simple',
  'draft-eagle3',
  'draft-mtp',
  'draft-dflash',
  'draft-dspark',
  'ngram-simple',
  'ngram-map-k',
  'ngram-map-k4v',
  'ngram-mod',
  'ngram-cache',
] as const

// llama.cpp evaluates enabled implementations in this order. The order in the
// comma-separated CLI value does not change this runtime priority.
export const SPECULATIVE_RUNTIME_PRIORITY = [
  'ngram-simple',
  'ngram-map-k',
  'ngram-map-k4v',
  'ngram-mod',
  'ngram-cache',
  'draft-simple',
  'draft-eagle3',
  'draft-mtp',
  'draft-dflash',
  'draft-dspark',
] as const

const runtimeRank = new Map<string, number>(
  SPECULATIVE_RUNTIME_PRIORITY.map((type, index) => [type, index]),
)

const rawTokens = (value: string | null | undefined): string[] => (
  (value || '')
    .split(',')
    .map(token => token.trim().toLowerCase())
    .filter(Boolean)
)

export function parseSpeculativeTypes(value: string | null | undefined): string[] {
  const tokens = rawTokens(value)
  if (tokens.includes('none') || tokens.includes('off')) return []
  return [...new Set(tokens)]
}

export function normalizeSpeculativeTypes(value: string | null | undefined): string {
  const tokens = rawTokens(value)
  if (tokens.length === 0) return ''
  if (tokens.includes('none') || tokens.includes('off')) return 'none'

  const unique = [...new Set(tokens)]
  return unique
    .map((type, inputIndex) => ({ type, inputIndex, rank: runtimeRank.get(type) }))
    .sort((left, right) => {
      if (left.rank !== undefined && right.rank !== undefined) return left.rank - right.rank
      if (left.rank !== undefined) return -1
      if (right.rank !== undefined) return 1
      return left.inputIndex - right.inputIndex
    })
    .map(candidate => candidate.type)
    .join(',')
}

export function hasSpeculativeType(
  value: string | null | undefined,
  type: string,
): boolean {
  return parseSpeculativeTypes(value).includes(type.trim().toLowerCase())
}

export function hasDraftSpeculativeType(value: string | null | undefined): boolean {
  return parseSpeculativeTypes(value).some(type => type.startsWith('draft-'))
}

export function isNgramOnlySpeculativeType(value: string | null | undefined): boolean {
  const types = parseSpeculativeTypes(value)
  return types.length > 0 && types.every(type => type.startsWith('ngram-'))
}

export function orderSpeculativeTypeOptions(types: readonly string[]): string[] {
  const unique = [...new Set(types.map(type => type.trim().toLowerCase()).filter(Boolean))]
  return unique.sort((left, right) => {
    if (left === 'none') return -1
    if (right === 'none') return 1
    const leftRank = runtimeRank.get(left)
    const rightRank = runtimeRank.get(right)
    if (leftRank !== undefined && rightRank !== undefined) return leftRank - rightRank
    if (leftRank !== undefined) return -1
    if (rightRank !== undefined) return 1
    return left.localeCompare(right)
  })
}
