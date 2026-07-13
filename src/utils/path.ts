/**
 * Cross-platform path utilities.
 * Internally all paths are normalized to POSIX-style (forward slash) for consistency.
 * Rust/Tauri returns OS-native paths (backslash on Windows); these utilities handle both.
 */

/** Normalize any platform path to POSIX style (forward slashes only). */
export function normalizePath(p: string): string {
  return p.replace(/\\/g, '/');
}

/** Get the file name (last segment) from a path. Cross-platform equivalent of path.basename(). */
export function pathBasename(p: string): string {
  return normalizePath(p).split('/').pop() || p;
}

/** Get the parent directory path. Cross-platform equivalent of path.dirname(). */
export function pathDirname(p: string): string {
  const n = normalizePath(p)
  if (n === '/' || n.endsWith(':/')) return n
  return n.replace(/\/[^/]*$/, '') || '/'
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
  return `${root}${joined}`
}

export function isPathWithinRoot(path: string, root: string): boolean {
  const normalizedPath = normalizePath(path)
  const normalizedRoot = normalizePath(root).replace(/\/+$/, '') || '/'
  return normalizedPath === normalizedRoot || normalizedPath.startsWith(`${normalizedRoot === '/' ? '' : normalizedRoot}/`)
}
