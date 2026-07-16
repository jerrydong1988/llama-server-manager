import { useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { marked } from 'marked'
import { driver } from 'driver.js'
import 'driver.js/dist/driver.css'
import { open as openExternal } from '@tauri-apps/plugin-shell'
import { ArrowUp, BookOpen, CheckCircle2, ChevronDown, Circle, Compass, PlayCircle } from 'lucide-react'
import { version } from '../../package.json'
import { useAppStore } from '../store'
import { formatMessage, useI18n } from '../i18n'
import { getGuideLabels } from '../i18n/pageLabels'
import { Button, InsetSurface, Surface } from './ui'
import { getGuideTourSteps } from './guide/guideTour'

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

function stripInAppTableOfContents(markdown: string): string {
  const output: string[] = []
  let skipping = false

  for (const line of markdown.split('\n')) {
    if (line.trim() === '## 目录 / Table of Contents') {
      skipping = true
      continue
    }

    if (skipping && line.startsWith('## ')) {
      skipping = false
    }

    if (!skipping) output.push(line)
  }

  return output.join('\n')
}

function normalizeGuideAssetPaths(markdown: string): string {
  return markdown.replace(/\(public\/docs\/guide\//g, '(/docs/guide/')
}

function renderMD(markdown: string): string {
  const processed = normalizeGuideAssetPaths(markdown).replace(/^## (.+)$/gm, (_, title: string) => {
    const id = title.toLowerCase().replace(/[^a-z0-9\u4e00-\u9fa5]+/g, '-')
    return `<h2 id="${id}">${title}</h2>`
  })
  const html = marked.parse(processed, { async: false }) as string
  return sanitize(html)
}

function sanitize(html: string): string {
  const allowedTags = new Set([
    'A', 'B', 'BLOCKQUOTE', 'BR', 'CODE', 'DIV', 'EM', 'H1', 'H2', 'H3', 'H4',
    'HR', 'I', 'IMG', 'LI', 'OL', 'P', 'PRE', 'SPAN', 'STRONG', 'TABLE',
    'TBODY', 'TD', 'TH', 'THEAD', 'TR', 'UL',
  ])
  const allowedAttrs = new Set(['href', 'src', 'alt', 'title', 'id', 'class', 'loading', 'decoding', 'width', 'height'])
  const template = document.createElement('template')
  template.innerHTML = html

  const cleanNode = (node: Node) => {
    for (const child of Array.from(node.childNodes)) {
      if (child.nodeType !== Node.ELEMENT_NODE) {
        continue
      }

      const element = child as HTMLElement
      if (!allowedTags.has(element.tagName)) {
        element.remove()
        continue
      }

      for (const attr of Array.from(element.attributes)) {
        const name = attr.name.toLowerCase()
        const value = attr.value.trim().toLowerCase()
        const unsafeProtocol = value.startsWith('javascript:') || value.startsWith('vbscript:')
        const unsafeHref = name === 'href' && value !== '' && !value.startsWith('#') && !value.startsWith('https://')
        const unsafeSrc = name === 'src' && value !== '' && !value.startsWith('/docs/guide/') && !value.startsWith('https://') && !value.startsWith('data:image/')
        if (!allowedAttrs.has(name) || name.startsWith('on') || unsafeProtocol || unsafeHref || unsafeSrc) {
          element.removeAttribute(attr.name)
        }
      }

      if (element.tagName === 'IMG') {
        element.setAttribute('loading', 'lazy')
        element.setAttribute('decoding', 'async')
      }

      cleanNode(element)
    }
  }

  cleanNode(template.content)
  return template.innerHTML
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
  const downloadTaskCount = useAppStore((state) => Object.keys(state.downloadTasks).length)
  const sysMetrics = useAppStore((state) => state.sysMetrics)
  const [html, setHtml] = useState('')
  const [toc, setToc] = useState<{ id: string; title: string }[]>([])
  const [activeSectionId, setActiveSectionId] = useState('')
  const [isChecklistOpen, setIsChecklistOpen] = useState(false)
  const [showBackToTop, setShowBackToTop] = useState(false)
  const contentRef = useRef<HTMLDivElement>(null)
  const scrollContainerRef = useRef<HTMLDivElement>(null)
  const tocScrollRef = useRef<HTMLDivElement>(null)
  const tocItemRefs = useRef(new Map<string, HTMLButtonElement>())
  const guideContent = useMemo(
    () => guideMd.replace(/v\d+\.\d+\.\d+/g, `v${version}`),
    [],
  )
  const inAppGuideContent = useMemo(
    () => stripInAppTableOfContents(guideContent),
    [guideContent],
  )
  const labels = useMemo(() => getGuideLabels(lang), [lang])

  const checklist = useMemo(() => {
    const hasRunningInstance = instances.some(instance => instance.status === 'running')
    const hasDownloadActivity = downloadQueue.length > 0 || downloadTaskCount > 0
    return [
      {
        id: 'model-dirs',
        title: labels.modelDirectories,
        detail: formatMessage(labels.modelDirectoryDetail, { directories: modelDirs.length, assets: models.length }),
        done: modelDirs.length > 0 && models.length > 0,
        tab: 'model-repo',
      },
      {
        id: 'engine-scan',
        title: labels.engineScan,
        detail: formatMessage(labels.engineScanDetail, { directories: engineDirs.length, engines: engines.length }),
        done: engines.length > 0,
        tab: 'engine',
      },
      {
        id: 'instance-create',
        title: labels.createInstance,
        detail: formatMessage(labels.instanceCount, { count: instances.length }),
        done: instances.length > 0,
        tab: 'instances',
      },
      {
        id: 'instance-start',
        title: labels.startInstance,
        detail: hasRunningInstance ? labels.runningInstance : labels.noRunningInstance,
        done: hasRunningInstance,
        tab: 'instances',
      },
      {
        id: 'downloads',
        title: labels.downloads,
        detail: formatMessage(labels.downloadDetail, { queue: downloadQueue.length, tasks: downloadTaskCount }),
        done: hasDownloadActivity,
        tab: 'downloads',
      },
      {
        id: 'performance',
        title: labels.performance,
        detail: sysMetrics ? labels.metricsAvailable : labels.inspectTelemetry,
        done: !!sysMetrics || instances.some(instance => instance.config.metrics && instance.status === 'running'),
        tab: 'perf',
      },
    ]
  }, [downloadQueue.length, downloadTaskCount, engineDirs.length, engines.length, instances, labels, modelDirs.length, models.length, sysMetrics])

  useEffect(() => {
    const nextToc = parseTOC(inAppGuideContent)
    setToc(nextToc)
    setActiveSectionId(nextToc[0]?.id ?? '')
    setHtml(renderMD(inAppGuideContent))
  }, [inAppGuideContent])

  useEffect(() => {
    const container = scrollContainerRef.current
    const headings = Array.from(contentRef.current?.querySelectorAll<HTMLElement>('h2[id]') ?? [])
    if (!container || headings.length === 0) return

    const updateScrollState = () => {
      const activationLine = container.getBoundingClientRect().top + 96
      let currentId = headings[0].id

      for (const heading of headings) {
        if (heading.getBoundingClientRect().top > activationLine) break
        currentId = heading.id
      }

      setActiveSectionId(currentId)
      setShowBackToTop(container.scrollTop > 480)
    }

    updateScrollState()
    container.addEventListener('scroll', updateScrollState, { passive: true })
    return () => container.removeEventListener('scroll', updateScrollState)
  }, [html])

  useEffect(() => {
    const container = tocScrollRef.current
    const item = tocItemRefs.current.get(activeSectionId)
    if (!container || !item) return

    const containerBounds = container.getBoundingClientRect()
    const itemBounds = item.getBoundingClientRect()
    if (itemBounds.top < containerBounds.top) {
      container.scrollTop -= containerBounds.top - itemBounds.top + 4
    } else if (itemBounds.bottom > containerBounds.bottom) {
      container.scrollTop += itemBounds.bottom - containerBounds.bottom + 4
    }
  }, [activeSectionId])

  const tourSteps = useMemo(() => getGuideTourSteps(lang), [lang])

  const handleTocClick = (id: string) => {
    const container = scrollContainerRef.current
    const element = contentRef.current?.querySelector(`#${id}`)
    if (container && element) {
      const containerBounds = container.getBoundingClientRect()
      const elementBounds = element.getBoundingClientRect()
      const targetTop = container.scrollTop + elementBounds.top - containerBounds.top - 24
      setActiveSectionId(id)
      container.scrollTo({ top: Math.max(0, targetTop), behavior: 'smooth' })
    }
  }

  const scrollToTop = () => {
    scrollContainerRef.current?.scrollTo({ top: 0, behavior: 'smooth' })
  }

  const handleContentClick = (event: ReactMouseEvent<HTMLDivElement>) => {
    const target = event.target instanceof Element ? event.target.closest('a') : null
    const href = target?.getAttribute('href')
    if (!href) return

    if (href.startsWith('#')) {
      event.preventDefault()
      handleTocClick(decodeURIComponent(href.slice(1)))
      return
    }

    if (href.startsWith('https://')) {
      event.preventDefault()
      void openExternal(href).catch((error) => console.warn(`Unable to open guide link: ${href}`, error))
    }
  }

  const startTour = async () => {
    let cancelled = false
    try {
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
        const element = await waitDOM(step.selector, 5000).catch((error) => {
          console.warn(`Guide tour target unavailable: ${step.selector}`, error)
          return null
        })
        if (!element) {
          continue
        }

        const isLast = index === tourSteps.length - 1

        await new Promise<void>((resolve) => {
          let settled = false
          const settle = () => {
            if (settled) return
            settled = true
            resolve()
          }
          const walkthrough = driver({
            animate: true,
            showProgress: true,
            steps: [{
              element: element as HTMLElement,
              popover: {
                title: step.title,
                description: step.description,
                doneBtnText: isLast ? labels.done : labels.next,
              },
            }],
            onCloseClick: () => {
              cancelled = true
              walkthrough.destroy()
              settle()
            },
            onDoneClick: () => {
              walkthrough.destroy()
              settle()
            },
            onDestroyed: settle,
          })
          walkthrough.drive()
        })

        if (cancelled) break
      }
    } finally {
      setActiveTab('guide')
    }
  }

  const completedChecklistCount = checklist.filter(item => item.done).length

  return (
    <div className="relative flex min-w-0 flex-1 overflow-hidden">
      <aside className="flex w-[260px] shrink-0 overflow-hidden border-r border-slate-200 bg-slate-100/80 p-3 dark:border-slate-800 dark:bg-slate-950/80 2xl:w-[300px] 2xl:p-4">
        <Surface className="flex min-h-0 flex-1 flex-col overflow-hidden p-4">
          <div className="mb-4 flex shrink-0 items-center gap-3">
            <div className="rounded-lg border border-blue-500/20 bg-blue-500/10 p-3 text-blue-600 dark:text-blue-300">
              <BookOpen className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-50">{labels.guide}</h2>
              <p className="mt-1 text-sm leading-5 text-slate-500 dark:text-slate-400">{labels.subtitle}</p>
            </div>
          </div>

          <Button
            onClick={() => void startTour()}
            variant="primary"
            className="mb-4 shrink-0"
            icon={<PlayCircle className="h-4 w-4" />}
          >
            {labels.startTour}
          </Button>

          <InsetSurface
            className={`flex min-h-0 flex-1 flex-col p-3 ${isChecklistOpen ? 'overflow-y-auto' : 'overflow-hidden'}`}
          >
            <button
              type="button"
              aria-controls="guide-setup-checklist"
              aria-expanded={isChecklistOpen}
              title={isChecklistOpen ? labels.collapseChecklist : labels.expandChecklist}
              onClick={() => setIsChecklistOpen(open => !open)}
              className="mb-3 flex w-full shrink-0 items-center gap-2 rounded-lg px-2 py-2 text-left text-xs font-medium text-slate-600 transition hover:bg-slate-200 hover:text-slate-900 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100"
            >
              <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate">{labels.checklistTitle}</span>
              <span className="shrink-0 tabular-nums text-slate-500">
                {completedChecklistCount}/{checklist.length}
              </span>
              <ChevronDown className={`h-4 w-4 shrink-0 transition-transform ${isChecklistOpen ? 'rotate-180' : ''}`} />
            </button>

            {isChecklistOpen && (
              <div id="guide-setup-checklist" className="mb-3 shrink-0 pr-1">
                <p className="mb-3 text-xs leading-5 text-slate-500">{labels.checklistDesc}</p>
                <div className="space-y-2">
                  {checklist.map(item => (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => setActiveTab(item.tab)}
                      className="flex w-full items-start gap-2 rounded-lg border border-slate-200 bg-white px-3 py-2 text-left transition hover:border-slate-300 hover:bg-slate-50 dark:border-slate-800 dark:bg-slate-950/40 dark:hover:border-slate-700 dark:hover:bg-slate-900"
                    >
                      {item.done ? (
                        <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-emerald-600 dark:text-emerald-300" />
                      ) : (
                        <Circle className="mt-0.5 h-4 w-4 shrink-0 text-slate-400 dark:text-slate-500" />
                      )}
                      <span className="min-w-0 flex-1">
                        <span className="flex min-w-0 items-center justify-between gap-2">
                          <span className="truncate text-sm text-slate-800 dark:text-slate-200">{item.title}</span>
                          <span className={`shrink-0 text-[11px] ${item.done ? 'text-emerald-600 dark:text-emerald-300' : 'text-slate-500'}`}>
                            {item.done ? labels.completed : labels.pending}
                          </span>
                        </span>
                        <span className="mt-1 block truncate text-xs text-slate-500">{item.detail}</span>
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            )}

            <div className="mb-2 flex shrink-0 items-center gap-2 px-2 text-xs font-medium text-slate-500">
              <Compass className="h-3.5 w-3.5" />
              {labels.contents}
            </div>
            <div ref={tocScrollRef} data-guide-toc className="min-h-24 flex-1 space-y-1 overflow-y-auto pr-1">
              {toc.map((item) => (
                <button
                  key={item.id}
                  ref={(element) => {
                    if (element) tocItemRefs.current.set(item.id, element)
                    else tocItemRefs.current.delete(item.id)
                  }}
                  type="button"
                  onClick={() => handleTocClick(item.id)}
                  aria-current={activeSectionId === item.id ? 'location' : undefined}
                  className={`w-full rounded-lg px-2.5 py-2 text-left text-sm transition ${
                    activeSectionId === item.id
                      ? 'bg-blue-600 font-medium text-white shadow-sm'
                      : 'text-slate-600 hover:bg-slate-200 hover:text-slate-900 dark:text-slate-300 dark:hover:bg-slate-800 dark:hover:text-slate-100'
                  }`}
                  title={item.title}
                >
                  {item.title}
                </button>
              ))}
            </div>
          </InsetSurface>
        </Surface>
      </aside>

      <div
        ref={scrollContainerRef}
        data-guide-scroll
        className="min-w-0 flex-1 overflow-y-auto bg-slate-100 px-4 py-5 dark:bg-slate-950 sm:px-6 2xl:px-8 2xl:py-6"
      >
        <Surface className="mx-auto max-w-5xl p-5 sm:p-8">
          <div
            ref={contentRef}
            className="guide-content prose max-w-none text-slate-800 dark:prose-invert dark:text-slate-200"
            onClick={handleContentClick}
            dangerouslySetInnerHTML={{ __html: html }}
          />
        </Surface>
      </div>

      {showBackToTop && (
        <button
          type="button"
          onClick={scrollToTop}
          title={labels.backToTop}
          aria-label={labels.backToTop}
          className="absolute bottom-6 right-6 z-10 flex h-10 w-10 items-center justify-center rounded-lg border border-blue-500 bg-blue-600 text-white shadow-lg transition hover:bg-blue-500 focus:outline-none focus:ring-2 focus:ring-blue-400 focus:ring-offset-2 focus:ring-offset-slate-100 dark:focus:ring-offset-slate-950"
        >
          <ArrowUp className="h-5 w-5" />
        </button>
      )}
    </div>
  )
}
