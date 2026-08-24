import { useEffect, useRef, useState } from 'react'
import { useAppStore, type MsFileEntry } from '../../store'
import type { DownloadProgress } from '../../store/types'
import { useI18n } from '../../i18n'
import { invokeApp as invoke } from '../../lib/ipc'
import { pathJoin } from '../../utils/path'
import { downloadFileKey } from './downloadPolicy'

type DownloadSource = 'modelscope' | 'huggingface'

const taskKey = (source: string, repoId: string, remotePath: string, saveDir: string) => (
  `${source}|${downloadFileKey(source, repoId, remotePath, saveDir)}`
)

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
      const tasksByKey = new Map(
        Object.values(allTasks).map(task => [
          taskKey(task.source, task.repoId, task.remotePath, task.saveDir),
          task,
        ]),
      )
      const completedTasks: DownloadProgress[] = []
      const resolvedDir = await invoke<string>('resolve_path', { path: browseSaveDir })
      const localPaths = result.map(file => pathJoin(resolvedDir, trimmedRepoId, file.path || file.name))
      let localSizes: Array<number | null> = []
      try {
        localSizes = await invoke<Array<number | null>>('check_local_files', { paths: localPaths })
      } catch {
        // Local discovery is an enhancement; repository results remain usable if it fails.
      }
      result.forEach((file, index) => {
        const localPath = pathJoin(resolvedDir, trimmedRepoId, file.path || file.name)
        const actualSize = localSizes[index]
        if (file.size <= 0 || actualSize !== file.size) return
        const existing = tasksByKey.get(taskKey(
          browseSource,
          trimmedRepoId,
          file.path || file.name,
          browseSaveDir,
        ))
        const completedAt = Date.now()
        completedTasks.push({
          id: existing?.id || crypto.randomUUID(),
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
        })
      })
      if (generation !== generationRef.current) return

      useAppStore.setState(state => {
        const tasks = { ...state.downloadTasks }
        const taskIdsByKey = new Map(
          Object.values(tasks).map(task => [
            taskKey(task.source, task.repoId, task.remotePath, task.saveDir),
            task.id,
          ]),
        )
        for (const completed of completedTasks) {
          const key = taskKey(completed.source, completed.repoId, completed.remotePath, completed.saveDir)
          const latestId = taskIdsByKey.get(key)
          const latest = latestId ? tasks[latestId] : undefined
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
          taskIdsByKey.set(key, id)
        }
        return { downloadTasks: tasks }
      })
    } catch (error: unknown) {
      if (generation === generationRef.current) {
        const message = error instanceof Error
          ? error.message
          : typeof error === 'string'
            ? error
            : t.modelRepo.networkError
        setStatus(`${t.modelRepo.queryFailed}${message}`)
      }
    } finally {
      if (generation === generationRef.current) setBrowsing(false)
    }
  }

  return { source, repoId, files, status, browsing, browsedRepoId, selectSource, changeRepoId, resetBrowse, browse }
}
