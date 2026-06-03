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
  return normalizePath(p).replace(/\/[^/]*$/, '');
}

/** Join path segments using forward slash. Strips leading/trailing slashes from intermediate segments. */
export function pathJoin(...segments: string[]): string {
  return segments
    .map(s => normalizePath(s).replace(/^\/+|\/+$/g, ''))
    .filter(s => s.length > 0)
    .join('/');
}
