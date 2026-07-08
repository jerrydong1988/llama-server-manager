import { useEffect, useMemo, useRef, useState } from 'react'
import { marked } from 'marked'
import { driver } from 'driver.js'
import 'driver.js/dist/driver.css'
import { BookOpen, CheckCircle2, Circle, Compass, PlayCircle } from 'lucide-react'
import { version } from '../../package.json'
import { useAppStore } from '../store'
import { useI18n } from '../i18n'
import { Button, InsetSurface, Surface } from './ui'

import guideMd from '../../GUIDE.md?raw'

function parseTOC(markdown: string): { id: string; title: string }[] {
  return markdown
    .split('\n')
    .filter((line) => line.startsWith('## '))
    .map((line) => {
      const title = line.replace(/^## /, '').trim()
      const id = title.toLowerCase().replace(/[^a-z0-9\u4e00-\u9fa5]+/g, '-')
      return { id, title }
    })
}

function renderMD(markdown: string): string {
  const processed = markdown.replace(/^## (.+)$/gm, (_, title: string) => {
    const id = title.toLowerCase().replace(/[^a-z0-9\u4e00-\u9fa5]+/g, '-')
    return `<h2 id="${id}">${title}</h2>`
  })
  const html = marked.parse(processed, { async: false }) as string
  return sanitize(html)
}

function sanitize(html: string): string {
  return html
    .replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, '')
    .replace(/<iframe\b[^<]*(?:(?!<\/iframe>)<[^<]*)*<\/iframe>/gi, '')
    .replace(/<object\b[^<]*(?:(?!<\/object>)<[^<]*)*<\/object>/gi, '')
    .replace(/<embed\b[^>]*>/gi, '')
    .replace(/\bon\w+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/gi, '')
    .replace(/javascript:/gi, '')
    .replace(/vbscript:/gi, '')
}

function waitDOM(selector: string, timeout: number): Promise<Element> {
  return new Promise((resolve, reject) => {
    const start = Date.now()
    const check = () => {
      const element = document.querySelector(selector)
      if (element) return resolve(element)
      if (Date.now() - start > timeout) return reject(new Error(`timeout: ${selector}`))
      requestAnimationFrame(check)
    }
    check()
  })
}

export default function GuidePage() {
  const { lang } = useI18n()
  const setActiveTab = useAppStore((state) => state.setActiveTab)
  const models = useAppStore((state) => state.models)
  const modelDirs = useAppStore((state) => state.modelDirs)
  const engines = useAppStore((state) => state.engines)
  const engineDirs = useAppStore((state) => state.engineDirs)
  const instances = useAppStore((state) => state.instances)
  const downloadQueue = useAppStore((state) => state.downloadQueue)
  const downloadTasks = useAppStore((state) => state.downloadTasks)
  const sysMetrics = useAppStore((state) => state.sysMetrics)
  const [html, setHtml] = useState('')
  const [toc, setToc] = useState<{ id: string; title: string }[]>([])
  const contentRef = useRef<HTMLDivElement>(null)
  const guideContent = useMemo(
    () => guideMd.replace(/v\d+\.\d+\.\d+/g, `v${version}`),
    [],
  )
  const zh = lang === 'zh-CN'
  const labels = {
    guide: zh ? '\u4f7f\u7528\u8bf4\u660e' : 'Guide',
    subtitle: zh
      ? '\u5feb\u901f\u67e5\u9605\u6587\u6863\uff0c\u6216\u76f4\u63a5\u8fdb\u5165\u4ea4\u4e92\u5f0f\u5f15\u5bfc\u3002'
      : 'Read the guide or jump straight into an interactive walkthrough.',
    startTour: zh ? '\u5f00\u59cb\u4ea4\u4e92\u5f0f\u5f15\u5bfc' : 'Start Interactive Tour',
    contents: zh ? '\u76ee\u5f55' : 'Contents',
    done: zh ? '\u5b8c\u6210' : 'Done',
    next: zh ? '\u4e0b\u4e00\u6b65' : 'Next',
    checklistTitle: zh ? '\u542f\u7528\u68c0\u67e5' : 'Setup Checklist',
    checklistDesc: zh ? '\u6839\u636e\u5f53\u524d\u914d\u7f6e\u548c\u8fd0\u884c\u72b6\u6001\u81ea\u52a8\u63a8\u65ad\u8fdb\u5ea6\u3002' : 'Derived from the current config and runtime state.',
    completed: zh ? '\u5df2\u5b8c\u6210' : 'Complete',
    pending: zh ? '\u5f85\u5904\u7406' : 'Pending',
  }

  const checklist = useMemo(() => {
    const hasRunningInstance = instances.some(instance => instance.status === 'running')
    const hasDownloadActivity = downloadQueue.length > 0 || Object.keys(downloadTasks).length > 0
    return [
      {
        id: 'model-dirs',
        title: zh ? '\u6a21\u578b\u76ee\u5f55' : 'Model directories',
        detail: zh
          ? `${modelDirs.length} \u4e2a\u76ee\u5f55\uff0c${models.length} \u4e2a\u8d44\u4ea7`
          : `${modelDirs.length} directories, ${models.length} assets`,
        done: modelDirs.length > 0 && models.length > 0,
        tab: 'model-repo',
      },
      {
        id: 'engine-scan',
        title: zh ? '\u5f15\u64ce\u626b\u63cf' : 'Engine scan',
        detail: zh
          ? `${engineDirs.length} \u4e2a\u76ee\u5f55\uff0c${engines.length} \u4e2a\u5f15\u64ce`
          : `${engineDirs.length} directories, ${engines.length} engines`,
        done: engines.length > 0,
        tab: 'engine',
      },
      {
        id: 'instance-create',
        title: zh ? '\u521b\u5efa\u5b9e\u4f8b' : 'Create an instance',
        detail: zh ? `${instances.length} \u4e2a\u5b9e\u4f8b` : `${instances.length} instances`,
        done: instances.length > 0,
        tab: 'instances',
      },
      {
        id: 'instance-start',
        title: zh ? '\u542f\u52a8\u5b9e\u4f8b' : 'Start an instance',
        detail: zh ? (hasRunningInstance ? '\u5df2\u6709\u8fd0\u884c\u4e2d\u5b9e\u4f8b' : '\u5c1a\u672a\u542f\u52a8') : (hasRunningInstance ? 'At least one instance is running' : 'No instance is running'),
        done: hasRunningInstance,
        tab: 'instances',
      },
      {
        id: 'downloads',
        title: zh ? '\u4e0b\u8f7d\u76ee\u5f55 / \u961f\u5217' : 'Download directory / queue',
        detail: zh
          ? `${downloadQueue.length} \u4e2a\u961f\u5217\u9879\uff0c${Object.keys(downloadTasks).length} \u4e2a\u4efb\u52a1`
          : `${downloadQueue.length} queue entries, ${Object.keys(downloadTasks).length} tasks`,
        done: hasDownloadActivity,
        tab: 'downloads',
      },
      {
        id: 'performance',
        title: zh ? '\u6027\u80fd\u76d1\u63a7' : 'Performance monitoring',
        detail: zh
          ? (sysMetrics ? '\u5df2\u6536\u5230\u7cfb\u7edf\u6307\u6807' : '\u542f\u52a8\u5b9e\u4f8b\u540e\u67e5\u770b\u9065\u6d4b')
          : (sysMetrics ? 'System metrics are available' : 'Start an instance to inspect telemetry'),
        done: !!sysMetrics || instances.some(instance => instance.config.metrics && instance.status === 'running'),
        tab: 'perf',
      },
    ]
  }, [downloadQueue.length, downloadTasks, engineDirs.length, engines.length, instances, modelDirs.length, models.length, sysMetrics, zh])

  useEffect(() => {
    setToc(parseTOC(guideContent))
    setHtml(renderMD(guideContent))
  }, [guideContent])

  const tourSteps = useMemo(
    () => (zh
      ? [
          { sel: '[data-guide="dashboard"]', title: '\u603b\u89c8', desc: '\u67e5\u770b\u7cfb\u7edf\u8d44\u6e90\u3001\u5b9e\u4f8b\u72b6\u6001\u4e0e\u6574\u4f53\u8fd0\u884c\u6982\u51b5\u3002', tab: 'dashboard' },
          { sel: '[data-guide="model-search"]', title: '\u6a21\u578b\u4ed3\u5e93', desc: '\u641c\u7d22\u672c\u5730\u6a21\u578b\u3001\u6295\u5f71\u5668\u4e0e\u6743\u91cd\u6587\u4ef6\u3002', tab: 'model-repo' },
          { sel: '[data-guide="download-source"]', title: '\u4e0b\u8f7d\u7ba1\u7406', desc: '\u4ece\u8fdc\u7a0b\u4ed3\u5e93\u6d4f\u89c8\u6587\u4ef6\u5e76\u7ba1\u7406\u672c\u5730\u4e0b\u8f7d\u961f\u5217\u3002', tab: 'downloads' },
          { sel: '[data-guide="engine-scan"]', title: '\u5f15\u64ce\u7ba1\u7406', desc: '\u626b\u63cf llama-server \u6240\u5728\u76ee\u5f55\u5e76\u8bbe\u7f6e\u9ed8\u8ba4\u5f15\u64ce\u3002', tab: 'engine' },
          { sel: '[data-guide="instance-create"]', title: '\u5b9e\u4f8b\u7ba1\u7406', desc: '\u521b\u5efa\u3001\u542f\u52a8\u3001\u91cd\u547d\u540d\u5e76\u7ef4\u62a4\u670d\u52a1\u5b9e\u4f8b\u3002', tab: 'instances' },
          { sel: '[data-guide="cluster-scan"]', title: '\u96c6\u7fa4\u7ba1\u7406', desc: '\u53d1\u73b0\u5c40\u57df\u7f51 Worker\uff0c\u5e76\u901a\u8fc7\u672c\u5730\u6216 SSH \u65b9\u5f0f\u542f\u52a8\u8282\u70b9\u3002', tab: 'cluster' },
          { sel: '[data-guide="perf-select"]', title: '\u6027\u80fd\u76d1\u63a7', desc: '\u89c2\u5bdf\u7cfb\u7edf\u8d44\u6e90\u3001slots \u72b6\u6001\u4e0e\u8fd1\u671f\u63a8\u7406\u541e\u5410\u3002', tab: 'perf' },
          { sel: '[data-guide="logs-clear"]', title: '\u670d\u52a1\u5668\u65e5\u5fd7', desc: '\u6309\u5b9e\u4f8b\u8fc7\u6ee4\u65e5\u5fd7\uff0c\u5e76\u4fdd\u6301\u5c3e\u90e8\u5b9e\u65f6\u8ddf\u968f\u3002', tab: 'logs' },
        ]
      : [
          { sel: '[data-guide="dashboard"]', title: 'Dashboard', desc: 'Review system resources, instance state, and overall health at a glance.', tab: 'dashboard' },
          { sel: '[data-guide="model-search"]', title: 'Model Repo', desc: 'Search local models, projectors, and matrix assets.', tab: 'model-repo' },
          { sel: '[data-guide="download-source"]', title: 'Downloads', desc: 'Browse remote repositories and manage the local download queue.', tab: 'downloads' },
          { sel: '[data-guide="engine-scan"]', title: 'Engines', desc: 'Scan runtime folders and choose a default engine.', tab: 'engine' },
          { sel: '[data-guide="instance-create"]', title: 'Instances', desc: 'Create, launch, rename, and manage server instances.', tab: 'instances' },
          { sel: '[data-guide="cluster-scan"]', title: 'Cluster', desc: 'Discover LAN workers and launch nodes locally or through SSH.', tab: 'cluster' },
          { sel: '[data-guide="perf-select"]', title: 'Performance', desc: 'Inspect system pressure, slots, and recent throughput.', tab: 'perf' },
          { sel: '[data-guide="logs-clear"]', title: 'Logs', desc: 'Filter live logs by instance and keep the tail pinned.', tab: 'logs' },
        ]),
    [lang],
  )

  const handleTocClick = (id: string) => {
    const element = contentRef.current?.querySelector(`#${id}`)
    if (element) {
      element.scrollIntoView({ behavior: 'smooth', block: 'start' })
    }
  }

  const startTour = async () => {
    for (let index = 0; index < tourSteps.length; index += 1) {
      const step = tourSteps[index]
      if (step.tab === 'config') {
        const store = useAppStore.getState()
        if (store.instances.length === 0) {
          continue
        }
        store.setActiveConfigInstanceId(store.instances[0].id)
      }

      setActiveTab(step.tab)
      const element = await waitDOM(step.sel, 5000).catch(() => null)
      if (!element) {
        continue
      }

      const isLast = index === tourSteps.length - 1

      await new Promise<void>((resolve) => {
        const walkthrough = driver({
          animate: true,
          showProgress: true,
          steps: [{
            element: element as HTMLElement,
            popover: {
              title: step.title,
              description: step.desc,
              doneBtnText: isLast
                ? (labels.done)
                : (labels.next),
            },
          }],
          onDestroyed: () => resolve(),
        })
        walkthrough.drive()
      })
    }
  }

  return (
    <div className="flex flex-1 overflow-hidden">
      <aside className="w-[300px] shrink-0 border-r border-slate-800 bg-slate-950/80 p-4">
        <Surface className="p-4">
          <div className="mb-4 flex items-center gap-3">
            <div className="rounded-2xl border border-blue-500/20 bg-blue-500/10 p-3 text-blue-300">
              <BookOpen className="h-5 w-5" />
            </div>
            <div>
              <h2 className="text-lg font-semibold text-slate-50">{labels.guide}</h2>
              <p className="mt-1 text-sm text-slate-400">{labels.subtitle}</p>
            </div>
          </div>

          <Button
            onClick={() => void startTour()}
            variant="primary"
            className="mb-4"
            icon={<PlayCircle className="h-4 w-4" />}
          >
            {labels.startTour}
          </Button>

          <InsetSurface className="p-3">
            <div className="mb-4">
              <div className="mb-2 flex items-center gap-2 text-xs uppercase tracking-[0.14em] text-slate-500">
                <CheckCircle2 className="h-3.5 w-3.5" />
                {labels.checklistTitle}
              </div>
              <p className="mb-3 text-xs leading-5 text-slate-500">{labels.checklistDesc}</p>
              <div className="space-y-2">
                {checklist.map(item => (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setActiveTab(item.tab)}
                    className="flex w-full items-start gap-2 rounded-lg border border-slate-800 bg-slate-950/40 px-3 py-2 text-left transition hover:border-slate-700 hover:bg-slate-900"
                  >
                    {item.done ? (
                      <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-emerald-300" />
                    ) : (
                      <Circle className="mt-0.5 h-4 w-4 shrink-0 text-slate-500" />
                    )}
                    <span className="min-w-0 flex-1">
                      <span className="flex min-w-0 items-center justify-between gap-2">
                        <span className="truncate text-sm text-slate-200">{item.title}</span>
                        <span className={`shrink-0 text-[11px] ${item.done ? 'text-emerald-300' : 'text-slate-500'}`}>
                          {item.done ? labels.completed : labels.pending}
                        </span>
                      </span>
                      <span className="mt-1 block truncate text-xs text-slate-500">{item.detail}</span>
                    </span>
                  </button>
                ))}
              </div>
            </div>

            <div className="mb-3 flex items-center gap-2 text-xs uppercase tracking-[0.14em] text-slate-500">
              <Compass className="h-3.5 w-3.5" />
              {labels.contents}
            </div>
            <div className="space-y-1">
              {toc.map((item) => (
                <button
                  key={item.id}
                  onClick={() => handleTocClick(item.id)}
                  className="w-full rounded-lg px-2.5 py-2 text-left text-sm text-slate-300 transition hover:bg-slate-800 hover:text-slate-100"
                  title={item.title}
                >
                  {item.title}
                </button>
              ))}
            </div>
          </InsetSurface>
        </Surface>
      </aside>

      <div className="flex-1 overflow-y-auto bg-slate-950 px-8 py-6">
        <Surface className="mx-auto max-w-5xl p-8">
          <div ref={contentRef} className="guide-content prose prose-invert max-w-none text-slate-200" dangerouslySetInnerHTML={{ __html: html }} />
        </Surface>
      </div>
    </div>
  )
}
