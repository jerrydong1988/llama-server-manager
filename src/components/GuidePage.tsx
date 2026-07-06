import { useEffect, useMemo, useRef, useState } from 'react'
import { marked } from 'marked'
import { driver } from 'driver.js'
import 'driver.js/dist/driver.css'
import { BookOpen, Compass, PlayCircle } from 'lucide-react'
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
  }

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
