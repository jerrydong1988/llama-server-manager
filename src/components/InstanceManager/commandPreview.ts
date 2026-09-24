import { maskCommandArguments } from '../../store/commandFormatting'

const categories = [
  ['model', /^(?:-m|-a|--model|--model-url|--alias|--mmproj(?:-.+)?|--lora(?:-.+)?|--chat-template(?:-.+)?|--jinja|--no-jinja|--embedding|--pooling|--reranking)$/],
  ['context', /^(?:-c|-ctk|-ctv|-cram|--ctx-size|--keep|--cache(?:-.+)?|--no-cache(?:-.+)?|--kv(?:-.+)?|--no-kv(?:-.+)?|--context-shift|--no-context-shift|--rope(?:-.+)?|--yarn(?:-.+)?|--swa(?:-.+)?)$/],
  ['performance', /^(?:-ngl|-t|-tb|-b|-ub|-np|-cb|-fa|-lm|-lzm|--n-gpu-layers|--threads(?:-batch)?|--batch-size|--ubatch-size|--parallel|--cont-batching|--no-cont-batching|--flash-attn|--load-mode|--lazy-mode|--mlock|--mmap|--no-mmap|--direct-io|--numa|--fit(?:-.+)?|--device|--split-mode|--tensor-split|--main-gpu|--cpu(?:-.+)?|--perf|--no-perf)$/],
  ['reasoning', /^(?:--reasoning(?:-.+)?|-n|--predict)$/],
  ['sampling', /^(?:-s|--seed|--temp|--top-k|--top-p|--top-n-sigma|--min-p|--repeat(?:-.+)?|--presence-penalty|--frequency-penalty|--dry(?:-.+)?|--xtc(?:-.+)?|--mirostat(?:-.+)?|--typical|--dynatemp(?:-.+)?|--samplers|--sampler-seq|--logit-bias|--grammar(?:-.+)?|--json-schema(?:-.+)?)$/],
  ['speculative', /^(?:-md|-ngld|--model-draft|--n-gpu-layers-draft|--spec(?:-.+)?|--draft(?:-.+)?|--lookup(?:-.+)?)$/],
  ['network', /^(?:--host|--port|--api-key(?:-file)?|--hf-token|-hft|--metrics|--props|--slots|--no-slots|--slot(?:-.+)?|--ssl(?:-.+)?|--api-prefix|--path|--threads-http|--ui|--no-ui|--ui-config(?:-.+)?|--timeout|--rpc)$/],
] as const

export type CommandCategory = typeof categories[number][0] | 'other'
export type CommandPreviewRow = { index: number; flag: string | null; values: string[] }

/** Read argv directly: never split or unquote a shell command for the preview. */
export function buildCommandPreview(command: string[]) {
  const maskedCommand = maskCommandArguments(command)
  const grouped = new Map<CommandCategory, CommandPreviewRow[]>()
  let current: CommandPreviewRow | undefined
  let positional = false
  for (let index = 1; index < maskedCommand.length; index++) {
    const argument = maskedCommand[index]
    if (!positional && (/^-{1,2}[a-zA-Z]/.test(argument) || argument === '--')) {
      const equals = argument.indexOf('=')
      const flag = equals < 0 ? argument : argument.slice(0, equals)
      const category = categories.find(([, pattern]) => pattern.test(flag))?.[0] ?? 'other'
      current = { index, flag, values: equals < 0 ? [] : [argument.slice(equals + 1)] }
      const rows = grouped.get(category)
      if (rows) rows.push(current)
      else grouped.set(category, [current])
      positional = argument === '--'
    } else if (current) {
      current.values.push(argument)
    } else {
      current = { index, flag: null, values: [argument] }
      grouped.set('other', [current])
    }
  }
  const order: CommandCategory[] = [...categories.map(([id]) => id), 'other']
  return {
    executable: maskedCommand[0] ?? '',
    maskedCommand,
    hasSecrets: command.some((argument, index) => argument !== maskedCommand[index]),
    groups: order.flatMap(id => grouped.has(id) ? [{ id, rows: grouped.get(id)! }] : []),
  }
}
