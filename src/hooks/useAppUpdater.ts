import { useCallback, useEffect, useRef, useState } from 'react'
import { BundleType, getBundleType } from '@tauri-apps/api/app'
import { confirm, message } from '@tauri-apps/plugin-dialog'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater'
import { invokeApp as invoke } from '../lib/ipc'

export interface AppUpdateInfo {
  latest_version: string
  progress: number | null
  busy: boolean
}

interface AppUpdaterCopy {
  updateReadyTitle: string
  updateInstallDescription: string
  updateActiveWorkloadsDescription: string
  updateInstallNow: string
  updateLater: string
  updateFailedTitle: string
  updateFailedDescription: string
}

interface VerifiedUpdaterPlatform {
  url: string
  signature: string
  sha256: string
}

interface VerifiedUpdaterRelease {
  version: string
  releaseTag: string
  sourceSha: string
  releaseCounter: number
  target: string
  platform: VerifiedUpdaterPlatform
}

function requireUpdaterPlatform(rawJson: Record<string, unknown>, target: string): VerifiedUpdaterPlatform {
  const platforms = rawJson.platforms
  if (!platforms || typeof platforms !== 'object' || Array.isArray(platforms)) {
    throw new Error('Updater manifest has no platform map')
  }
  const platform = (platforms as Record<string, unknown>)[target]
  if (!platform || typeof platform !== 'object' || Array.isArray(platform)) {
    throw new Error('Updater manifest has no matching platform')
  }
  const record = platform as Record<string, unknown>
  if (typeof record.url !== 'string' || typeof record.signature !== 'string' || typeof record.sha256 !== 'string') {
    throw new Error('Updater platform identity is incomplete')
  }
  return { url: record.url, signature: record.signature, sha256: record.sha256 }
}

function assertVerifiedUpdateTuple(
  update: Update,
  verified: VerifiedUpdaterRelease,
  target: string,
) {
  const raw = update.rawJson
  const platform = requireUpdaterPlatform(raw, target)
  if (
    update.version !== verified.version
    || raw.version !== verified.version
    || raw.release_tag !== verified.releaseTag
    || raw.source_sha !== verified.sourceSha
    || raw.release_counter !== verified.releaseCounter
    || verified.target !== target
    || platform.url !== verified.platform.url
    || platform.signature !== verified.platform.signature
    || platform.sha256 !== verified.platform.sha256
  ) {
    throw new Error('Updater metadata does not match the authenticated release envelope')
  }
}

const UPDATE_RETRY_DELAYS_MS = [5_000, 30_000, 120_000]
const UPDATE_PERIODIC_CHECK_MS = 6 * 60 * 60_000

export function useAppUpdater(hasRunningInstances: boolean, copy: AppUpdaterCopy) {
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null)
  const [checking, setChecking] = useState(false)
  const [checkError, setCheckError] = useState<string | null>(null)
  const updateRef = useRef<Update | null>(null)
  const checkInFlightRef = useRef(false)
  const installingRef = useRef(false)
  const disposedRef = useRef(false)

  const checkForUpdate = useCallback(async () => {
    if (checkInFlightRef.current || installingRef.current) return true
    checkInFlightRef.current = true
    setChecking(true)
    setCheckError(null)
    try {
      const bundleType = await getBundleType()
      const update = await (async () => {
        if (bundleType === BundleType.Deb || bundleType === BundleType.Rpm) return null
        const requestedTarget = bundleType === BundleType.Nsis
          ? 'windows-x86_64-nsis'
          : bundleType === BundleType.Msi
            ? 'windows-x86_64-msi'
            : undefined
        const candidate = await check({ timeout: 15_000, target: requestedTarget })
        if (!candidate) return null
        const target = requestedTarget ?? (() => {
          const platforms = candidate.rawJson.platforms
          if (!platforms || typeof platforms !== 'object' || Array.isArray(platforms)) {
            throw new Error('Updater manifest has no platform map')
          }
          const darwinTargets = Object.keys(platforms).filter(name => /^darwin-(aarch64|x86_64)$/.test(name))
          if (darwinTargets.length !== 1) throw new Error('Updater manifest has no unique macOS target')
          return darwinTargets[0]
        })()
        const verified = await invoke<VerifiedUpdaterRelease>('verify_updater_release', { target })
        try {
          assertVerifiedUpdateTuple(candidate, verified, target)
          return candidate
        } catch (error) {
          await candidate.close().catch(() => {})
          throw error
        }
      })()

      if (disposedRef.current) {
        if (update) void update.close().catch(() => {})
        return true
      }
      const previous = updateRef.current
      updateRef.current = update
      if (previous && previous !== update) void previous.close().catch(() => {})
      setUpdateInfo(update
        ? { latest_version: update.version, progress: null, busy: false }
        : null)
      return true
    } catch (error) {
      if (!disposedRef.current) setCheckError(String(error))
      return false
    } finally {
      checkInFlightRef.current = false
      if (!disposedRef.current) setChecking(false)
    }
  }, [])

  useEffect(() => {
    disposedRef.current = false
    let retryTimer: number | undefined
    const runWithRetry = async (attempt: number) => {
      const succeeded = await checkForUpdate()
      if (disposedRef.current || succeeded || attempt >= UPDATE_RETRY_DELAYS_MS.length) return
      retryTimer = window.setTimeout(
        () => void runWithRetry(attempt + 1),
        UPDATE_RETRY_DELAYS_MS[attempt],
      )
    }

    void runWithRetry(0)
    const periodicTimer = window.setInterval(
      () => void runWithRetry(0),
      UPDATE_PERIODIC_CHECK_MS,
    )

    return () => {
      disposedRef.current = true
      if (retryTimer !== undefined) window.clearTimeout(retryTimer)
      window.clearInterval(periodicTimer)
      const update = updateRef.current
      updateRef.current = null
      if (update) void update.close().catch(() => {})
    }
  }, [checkForUpdate])

  const installUpdate = async () => {
    const update = updateRef.current
    if (!update || updateInfo?.busy || installingRef.current || checkInFlightRef.current) return
    installingRef.current = true
    setUpdateInfo(current => current ? { ...current, busy: true } : current)
    try {
      const proxyStatus = await invoke<{ running: boolean }>('get_proxy_status').catch(() => null)
      const hasActiveWorkloads = hasRunningInstances || proxyStatus?.running === true
      const accepted = await confirm(
        hasActiveWorkloads
          ? copy.updateActiveWorkloadsDescription
          : copy.updateInstallDescription,
        {
          title: copy.updateReadyTitle,
          kind: hasActiveWorkloads ? 'warning' : 'info',
          okLabel: copy.updateInstallNow,
          cancelLabel: copy.updateLater,
        },
      )
      if (!accepted) {
        installingRef.current = false
        setUpdateInfo(current => current ? { ...current, progress: null, busy: false } : current)
        return
      }

      let downloaded = 0
      let contentLength = 0
      const onDownloadEvent = (event: DownloadEvent) => {
        if (event.event === 'Started') {
          downloaded = 0
          contentLength = event.data.contentLength ?? 0
          setUpdateInfo(current => current
            ? { ...current, progress: contentLength > 0 ? 0 : null, busy: true }
            : current)
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength
          const progress = contentLength > 0
            ? Math.min(99, Math.round((downloaded / contentLength) * 100))
            : null
          setUpdateInfo(current => current ? { ...current, progress, busy: true } : current)
        } else if (event.event === 'Finished') {
          setUpdateInfo(current => current ? { ...current, progress: 100, busy: true } : current)
        }
      }

      setUpdateInfo(current => current ? { ...current, progress: 0, busy: true } : current)
      await update.downloadAndInstall(onDownloadEvent, { timeout: 15 * 60_000 })
      await relaunch()
    } catch (error) {
      installingRef.current = false
      setUpdateInfo(current => current ? { ...current, progress: null, busy: false } : current)
      await message(`${copy.updateFailedDescription}\n\n${String(error)}`, {
        title: copy.updateFailedTitle,
        kind: 'error',
      })
    }
  }

  return { updateInfo, installUpdate, checkForUpdate, checking, checkError }
}
