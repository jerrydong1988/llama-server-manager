function normalizeHost(host: string): string {
  const trimmed = host.trim()
  return trimmed.startsWith('[') && trimmed.endsWith(']') ? trimmed.slice(1, -1) : trimmed
}

export function formatHostPort(host: string, port: number): string {
  const normalized = normalizeHost(host)
  const authorityHost = normalized.includes(':') ? `[${normalized}]` : normalized
  return `${authorityHost}:${port}`
}

export function parseHostPort(value: string, defaultPort: number): { host: string; port: number } {
  const trimmed = value.trim()
  const bracketed = /^\[([^\]]+)](?::(\d+))?$/.exec(trimmed)
  if (bracketed) {
    return { host: bracketed[1], port: bracketed[2] ? Number(bracketed[2]) : defaultPort }
  }

  const colonCount = (trimmed.match(/:/g) || []).length
  if (colonCount === 1) {
    const separator = trimmed.lastIndexOf(':')
    const portText = trimmed.slice(separator + 1)
    if (/^\d+$/.test(portText)) {
      return { host: trimmed.slice(0, separator), port: Number(portText) }
    }
  }

  return { host: normalizeHost(trimmed), port: defaultPort }
}

export function httpUrl(host: string, port: number, path = ''): string {
  const suffix = path && !path.startsWith('/') ? `/${path}` : path
  return `http://${formatHostPort(host, port)}${suffix}`
}
