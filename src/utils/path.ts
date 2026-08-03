/**
 * Cross-platform path utilities.
 *
 * Rust canonical paths on Windows use the extended `\\?\` namespace while persisted legacy
 * configuration and user input commonly use drive or UNC paths. Keep display paths readable,
 * but derive every comparison from one stable identity so those representations stay equivalent.
 */

const WINDOWS_DRIVE_ROOT = /^[A-Za-z]:\/$/
const WINDOWS_DRIVE_PATH = /^[A-Za-z]:(?:\/|$)/
const WINDOWS_HOST = typeof navigator !== 'undefined' && /windows/i.test(navigator.userAgent)

const stripWindowsNamespace = (value: string): string => {
  if (/^\/\/\?\/UNC\//i.test(value)) return `//${value.slice(8)}`
  if (/^\/\/\?\//i.test(value)) return value.slice(4)
  return value
}

const collapsePathSegments = (value: string): string => {
  const hasUncRoot = value.startsWith('//')
  const hasPosixRoot = !hasUncRoot && value.startsWith('/')
  const root = hasUncRoot ? '//' : hasPosixRoot ? '/' : ''
  const segments: string[] = []

  for (const segment of value.slice(root.length).split('/')) {
    if (!segment || segment === '.') continue
    if (segment === '..') {
      if (segments.length > 0 && segments[segments.length - 1] !== '..') {
        segments.pop()
      } else if (!root) {
        segments.push(segment)
      }
      continue
    }
    segments.push(segment)
  }

  let normalized = `${root}${segments.join('/')}`
  if (!normalized && root) normalized = root
  if (!normalized && value.trim()) normalized = '.'
  if (WINDOWS_DRIVE_PATH.test(normalized) && /^[A-Za-z]:\/$/.test(value)) {
    normalized = `${normalized.replace(/\/$/, '')}/`
  }
  return normalized
}

/** Normalize separators, namespace prefixes, redundant segments, and trailing separators. */
export function normalizePath(p: string): string {
  const normalized = collapsePathSegments(stripWindowsNamespace(p.replace(/\\/g, '/')))
  if (normalized.length <= 1 || WINDOWS_DRIVE_ROOT.test(normalized)) return normalized
  return normalized.replace(/\/+$/, '')
}

/** Return a stable key for equality and containment checks without changing stored/display values. */
export function pathComparisonKey(p: string): string {
  const raw = p.trim()
  const normalized = normalizePath(raw)
  const isWindowsPath = WINDOWS_HOST
    || WINDOWS_DRIVE_PATH.test(normalized)
    || normalized.startsWith('//')
    || raw.includes('\\')
    || /^(?:\\\\|\/\/)(?:\?|\.)[\\/]/.test(raw)
  return isWindowsPath ? normalized.toLowerCase() : normalized
}

export function pathsEqual(left: string, right: string): boolean {
  return pathComparisonKey(left) === pathComparisonKey(right)
}

export function dedupePaths(paths: Iterable<string>): string[] {
  const seen = new Set<string>()
  const result: string[] = []
  for (const path of paths) {
    if (!path.trim()) continue
    const key = pathComparisonKey(path)
    if (seen.has(key)) continue
    seen.add(key)
    result.push(path)
  }
  return result
}

/** Get the file name (last segment) from a path. Cross-platform equivalent of path.basename(). */
export function pathBasename(p: string): string {
  return normalizePath(p).split('/').pop() || p;
}

/** Get the parent directory path. Cross-platform equivalent of path.dirname(). */
export function pathDirname(p: string): string {
  const n = normalizePath(p)
  if (n === '/' || n.endsWith(':/')) return n
  const separatorIndex = n.lastIndexOf('/')
  if (separatorIndex < 0) return '.'
  if (separatorIndex === 0) return '/'
  if (separatorIndex === 2 && WINDOWS_DRIVE_PATH.test(n)) return n.slice(0, 3)
  return n.slice(0, separatorIndex)
}

/** Join path segments using forward slash. Strips leading/trailing slashes from intermediate segments. */
export function pathJoin(...segments: string[]): string {
  const normalized = segments
    .map(normalizePath)
    .filter(s => s.length > 0)
  const first = normalized[0] || ''
  const root = first.startsWith('//') ? '//' : first.startsWith('/') ? '/' : ''
  const joined = normalized
    .map(s => s.replace(/^\/+|\/+$/g, ''))
    .filter(s => s.length > 0)
    .join('/')
  return normalizePath(`${root}${joined}`)
}

export function isPathWithinRoot(path: string, root: string): boolean {
  if (!root.trim()) return false
  const normalizedPath = pathComparisonKey(path)
  const normalizedRoot = pathComparisonKey(root)
  if (!normalizedRoot) return false
  if (normalizedRoot === '.') {
    return normalizedPath === '.'
      || Boolean(
        normalizedPath
        && normalizedPath !== '..'
        && !normalizedPath.startsWith('../')
        && !normalizedPath.startsWith('/')
        && !/^[a-z]:\//i.test(normalizedPath),
      )
  }
  const childPrefix = normalizedRoot.endsWith('/') ? normalizedRoot : `${normalizedRoot}/`
  return normalizedPath === normalizedRoot || normalizedPath.startsWith(childPrefix)
}
