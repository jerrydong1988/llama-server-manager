export type InstanceTestState = 'checking' | `ok:${string}` | `error:${string}`

type InstanceTestResultProps = {
  result?: InstanceTestState
  checkingLabel: string
}

export const InstanceTestResult = ({ result, checkingLabel }: InstanceTestResultProps) => {
  if (!result) return null
  if (result === 'checking') {
    return <span className="max-w-[180px] truncate text-xs text-blue-500">{checkingLabel}</span>
  }
  if (result.startsWith('ok:')) {
    const text = result.slice(3)
    return <span className="max-w-[180px] truncate text-xs text-emerald-500" title={text}>{text}</span>
  }
  const text = result.slice(6)
  return <span className="max-w-[180px] truncate text-xs text-rose-500" title={text}>{text}</span>
}
