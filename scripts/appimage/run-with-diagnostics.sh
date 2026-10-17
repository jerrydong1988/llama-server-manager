#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
shopt -s nullglob
images=(*.AppImage)
if [ "${#images[@]}" -ne 1 ]; then
  echo 'Keep exactly one candidate AppImage beside this script.' >&2
  exit 1
fi
if [ "${1:-}" = --extract ]; then
  export APPIMAGE_EXTRACT_AND_RUN=1
  shift
fi
log="appimage-test-$(date +%Y%m%d-%H%M%S).log"
{
  date -Is
  uname -m
  cat /etc/os-release
  printf 'session=%s desktop=%s\n' "${XDG_SESSION_TYPE:-unknown}" "${XDG_CURRENT_DESKTOP:-unknown}"
  printf 'GDK_BACKEND=%s WEBKIT_DISABLE_DMABUF_RENDERER=%s LIBGL_ALWAYS_SOFTWARE=%s\n' "${GDK_BACKEND:-unset}" "${WEBKIT_DISABLE_DMABUF_RENDERER:-unset}" "${LIBGL_ALWAYS_SOFTWARE:-unset}"
  if command -v lspci >/dev/null; then lspci -k | grep -A3 -E 'VGA|3D|Display' || true; fi
} | tee "$log"
printf 'Starting %s; log: %s\n' "${images[0]}" "$log"
"./${images[0]}" "$@" 2>&1 | tee -a "$log"
