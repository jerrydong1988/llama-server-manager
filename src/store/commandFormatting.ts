export function maskStartupCommandSecrets(cmdStr: string): string {
  return cmdStr.replace(
    /(--api-key(?:\s+|=))(?:"[^"]*"|'[^']*'|[^\s]+)/g,
    '$1********',
  )
}

export function formatStartupCommand(cmdStr: string): string {
  const tokens = cmdStr.match(/(?:[^\s"]+|"[^"]*")+/g) || []
  const exeName = (tokens[0] || '').split(/[\\/]/).pop() || tokens[0] || ''

  const groups: Record<string, string[]> = {
    Model: [],
    Reasoning: [],
    Performance: [],
    Cache: [],
    Memory: [],
    Sampling: [],
    Speculative: [],
    Multimodal: [],
    Network: [],
    Other: [],
  }

  const classify = (flag: string): string => {
    if (/^-m$|^-a$|^--alias|^--mmproj|^--lora|^--chat-template|^--chat-template-file|^--grammar\b|^--skip-chat|^--jinja|^--models-dir|^--models-preset|^--models-max|^--models-autoload|^--tools/.test(flag)) return 'Model'
    if (/^--reasoning|^--reasoning-budget/.test(flag)) return 'Reasoning'
    if (/^-ngl|^-t$|^-tb$|^-b$|^-ub$|^-np|^-cb|^--threads|^--batch/.test(flag)) return 'Performance'
    if (/^-c$|^--ctx|^--keep|^-cram|^--cache-ram|^--cache-reuse|^--cache-idle|^--kv-unified|^--warmup|^--no-cache|^--override-kv|^--rope-scaling|^--rope-scale|^--rope-freq-base|^--rope-freq-scale|^--yarn-ext-factor|^--yarn-attn-factor|^--yarn-beta|^--no-context-shift|^--swa/.test(flag)) return 'Cache'
    if (/^-fa|^--mlock|^--no-mmap|^--numa|^--check-tensors|^--fit/.test(flag)) return 'Memory'
    if (/^-n$|^--temp$|^--top-k|^--top-p|^--top-n-sigma|^--min-p|^--repeat|^-s$|^--seed|^--presence|^--frequency|^--ignore-eos|^--json-schema|^--mirostat|^--xtc|^--dynatemp|^--typical|^--dry|^--adaptive|^--logit-bias|^--samplers\b|^--sampler-seq|^-bs|^--backend-sampling|^-sp$|^--special|^--reverse-prompt|^--spm-infill/.test(flag)) return 'Sampling'
    if (/^--spec|^-md$|^-ngld|-lcs|-lcd|^--lookup|^--draft/.test(flag)) return 'Speculative'
    if (/^--image|^--mmproj-url|^--mmproj-auto|^--embedding|^--pooling|^--reranking|^--embd-normalize|^--tags|^--media/.test(flag)) return 'Multimodal'
    if (/^--host|^--port|^--api-key|^--ssl|^--path|^--api-prefix|^--no-ui|^--threads-http|^--metrics|^--props|^--slot|^--ui-config|^--sleep-idle|^--verbose/.test(flag)) return 'Network'
    return 'Other'
  }

  for (let i = 1; i < tokens.length; i++) {
    const token = tokens[i].replace(/^"|"$/g, '')
    if (!token.startsWith('-')) continue

    const category = classify(token)
    const nextIsValue = i + 1 < tokens.length && (
      !tokens[i + 1].startsWith('-') || /^-\d+(\.\d+)?$/.test(tokens[i + 1])
    )

    if (nextIsValue) {
      const value = tokens[i + 1].replace(/^"|"$/g, '')
      const shortValue = (token === '-m' || token === '-md' || token === '--mmproj' || token === '--lora')
        ? value.split(/[\\/]/).pop() || value
        : value
      groups[category].push(`${token} ${shortValue}`)
      i++
    } else {
      groups[category].push(token)
    }
  }

  const lines: string[] = []
  lines.push(`Startup Command (EXE: ${exeName})`)

  for (const [label, args] of Object.entries(groups)) {
    if (args.length > 0) {
      lines.push(`- [${label}] ${args.join('  ')}`)
    }
  }

  const pathFlags = new Set(['-m', '-md', '--mmproj', '--lora', '--lora-init-without-apply'])
  const shortParts: string[] = []
  for (let i = 1; i < tokens.length; i++) {
    const token = tokens[i].replace(/^"|"$/g, '')
    if (pathFlags.has(token) && i + 1 < tokens.length) {
      const value = tokens[i + 1].replace(/^"|"$/g, '')
      shortParts.push(`${token} "${value.split(/[\\/]/).pop() || value}"`)
      i++
    } else {
      shortParts.push(tokens[i])
    }
  }

  lines.push('')
  lines.push(`Full: ${shortParts.join(' ')}`)

  return lines.join('\n')
}
