import { useEffect, useRef, useState } from 'react'
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

export function useAppUpdater(hasRunningInstances: boolean, copy: AppUpdaterCopy) {
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null)
  const updateRef = useRef<Update | null>(null)

  useEffect(() => {
    let disposed = false

    getBundleType()
      .then(bundleType => {
        if (bundleType === BundleType.Deb || bundleType === BundleType.Rpm) return null
        const target = bundleType === BundleType.Nsis
          ? 'windows-x86_64-nsis'
          : bundleType === BundleType.Msi
            ? 'windows-x86_64-msi'
            : undefined
        return check({ timeout: 15_000, target })
      })
      .then(update => {
        if (disposed) {
          if (update) void update.close().catch(() => {})
          return
        }
        updateRef.current = update
        if (update) {
          setUpdateInfo({ latest_version: update.version, progress: null, busy: false })
        }
      })
      .catch(() => {})

    return () => {
      disposed = true
      const update = updateRef.current
      updateRef.current = null
      if (update) void update.close().catch(() => {})
    }
  }, [])

  const installUpdate = async () => {
    const update = updateRef.current
    if (!update || updateInfo?.busy) return

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
    if (!accepted) return

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
    try {
      await update.downloadAndInstall(onDownloadEvent, { timeout: 15 * 60_000 })
      await relaunch()
    } catch (error) {
      setUpdateInfo(current => current ? { ...current, progress: null, busy: false } : current)
      await message(`${copy.updateFailedDescription}\n\n${String(error)}`, {
        title: copy.updateFailedTitle,
        kind: 'error',
      })
    }
  }

  return { updateInfo, installUpdate }
}
