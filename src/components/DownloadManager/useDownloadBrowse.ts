import { useEffect, useRef, useState } from 'react'
import { useAppStore, type MsFileEntry } from '../../store'
import type { DownloadProgress } from '../../store/types'
import { useI18n } from '../../i18n'
import { invokeApp as invoke } from '../../lib/ipc'
import { forEachConcurrent } from '../../utils/async'
import { pathJoin, pathsEqual } from '../../utils/path'
import { LOCAL_FILE_CHECK_CONCURRENCY } from './downloadPolicy'

type DownloadSource = 'modelscope' | 'huggingface'

export function useDownloadBrowse(saveDir: string) {
  const { t } = useI18n()
  const browseModelscope = useAppStore(state => state.browseModelscope)
  const browseHuggingface = useAppStore(state => state.browseHuggingface)
  const [source, setSource] = useState<DownloadSource>('modelscope')
  const [repoId, setRepoId] = useState('')
  const [files, setFiles] = useState<MsFileEntry[]>([])
  const [status, setStatus] = useState('')
  const [browsing, setBrowsing] = useState(false)
  const [browsedRepoId, setBrowsedRepoId] = useState('')
  const generationRef = useRef(0)

  useEffect(() => () => {
    generationRef.current += 1
  }, [])

  const invalidate = () => {
    generationRef.current += 1
    setBrowsing(false)
  }

  const resetBrowse = () => {
    invalidate()
    setFiles([])
    setBrowsedRepoId('')
    setStatus('')
  }

  const selectSource = (nextSource: DownloadSource) => {
    resetBrowse()
    setSource(nextSource)
  }

  const changeRepoId = (nextRepoId: string) => {
    resetBrowse()
    setRepoId(nextRepoId)
  }

  const browse = async () => {
    if (!repoId.trim()) {
      setStatus(t.modelRepo.inputRepoId)
      return
    }

    const generation = ++generationRef.current
    const browseStartedAt = Date.now()
    const trimmedRepoId = repoId.trim()
    const browseSource = source
    const browseSaveDir = saveDir
    setBrowsing(true)
    setStatus(t.modelRepo.querying)

    try {
      const result = browseSource === 'modelscope'
        ? await browseModelscope(trimmedRepoId)
        : await browseHuggingface(trimmedRepoId)
      if (generation !== generationRef.current) return

      setFiles(result)
      setBrowsedRepoId(trimmedRepoId)
      setStatus(result.length === 0 ? t.modelRepo.notFound : `${t.modelRepo.found} ${result.length} ${t.modelRepo.files}`)

      const allTasks = useAppStore.getState().downloadTasks
      const completedTasks: DownloadProgress[] = []
      const resolvedDir = await invoke<string>('resolve_path', { path: browseSaveDir })
      await forEachConcurrent(result, LOCAL_FILE_CHECK_CONCURRENCY, async file => {
        const localPath = pathJoin(resolvedDir, trimmedRepoId, file.path || file.name)
        try {
          const checked = await invoke<{ taskId: string | null, size: number, managerOwned: boolean } | null>('check_local_file', {
            saveDir: browseSaveDir,
            repoId: trimmedRepoId,
            remotePath: file.path || file.name,
          })
          if (file.size <= 0 || checked?.size !== file.size) return
          const actualSize = checked.size
          const existing = Object.values(allTasks).find(task => (
            task.source === browseSource
            && task.repoId === trimmedRepoId
            && task.remotePath === (file.path || file.name)
            && pathsEqual(task.saveDir, browseSaveDir)
          ))
          const completedAt = Date.now()
          const observedId = `observed:${browseSource}:${resolvedDir}:${trimmedRepoId}:${file.path || file.name}`
          completedTasks.push({
            id: checked.taskId ?? observedId,
            fileName: file.name,
            remotePath: file.path || file.name,
            fileType: file.file_type,
            saveDir: browseSaveDir,
            repoId: trimmedRepoId,
            source: browseSource,
            downloaded: actualSize,
            total: file.size,
            speed: 0,
            status: 'completed',
            path: localPath,
            version: existing?.version ?? 0,
            createdAt: existing?.createdAt ?? completedAt,
            updatedAt: completedAt,
            completedAt: existing?.completedAt ?? completedAt,
            managerOwned: checked.managerOwned,
          })
        } catch {
          // Missing local files remain available for download.
        }
      })
      if (generation !== generationRef.current) return

      useAppStore.setState(state => {
        const tasks = { ...state.downloadTasks }
        for (const completed of completedTasks) {
          const latest = Object.values(tasks).find(task => (
            task.source === completed.source
            && task.repoId === completed.repoId
            && task.remotePath === completed.remotePath
            && pathsEqual(task.saveDir, completed.saveDir)
          ))
          if ((latest?.version ?? 0) > (completed.version ?? 0)) continue
          if (latest && (latest.updatedAt ?? 0) > browseStartedAt && latest.status !== 'completed') continue
          if (latest && ['active', 'queued', 'pausing'].includes(latest.status)) continue
          const id = latest?.id || completed.id
          tasks[id] = {
            ...latest,
            ...completed,
            id,
            version: Math.max(latest?.version ?? 0, completed.version ?? 0),
          }
        }
        return { downloadTasks: tasks }
      })
    } catch (error: unknown) {
      if (generation === generationRef.current) {
        setStatus(`${t.modelRepo.queryFailed}${typeof error === 'string' ? error : t.modelRepo.networkError}`)
      }
    } finally {
      if (generation === generationRef.current) setBrowsing(false)
    }
  }

  return { source, repoId, files, status, browsing, browsedRepoId, selectSource, changeRepoId, resetBrowse, browse }
}
