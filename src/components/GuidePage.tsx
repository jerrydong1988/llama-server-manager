import { useState, useEffect, useRef } from 'react'
import { marked } from 'marked'
import { driver } from 'driver.js'
import 'driver.js/dist/driver.css';
import { useAppStore } from '../store'

// Import GUIDE.md as raw string at compile time
import guideMd from '../../GUIDE.md?raw'

// Parse all ## headers for TOC
function parseTOC(md: string): { id: string; title: string }[] {
  return md.split('\n')
    .filter(l => l.startsWith('## '))
    .map(l => {
      const title = l.replace(/^## /, '').trim()
      const id = title.toLowerCase().replace(/[^a-z0-9\u4e00-\u9fa5]+/g, '-')
      return { id, title }
    })
}

// Convert Markdown to HTML, add id anchors to ## headers
function renderMD(md: string): string {
  const processed = md.replace(/^## (.+)$/gm, (_, title: string) => {
    const id = title.toLowerCase().replace(/[^a-z0-9\u4e00-\u9fa5]+/g, '-')
    return `<h2 id="${id}">${title}</h2>`
  })
  return marked.parse(processed, { async: false }) as string
}

// Wait for DOM element with timeout
function waitDOM(selector: string, timeout: number): Promise<Element> {
  return new Promise((resolve, reject) => {
    const start = Date.now()
    const check = () => {
      const el = document.querySelector(selector)
      if (el) return resolve(el)
      if (Date.now() - start > timeout) return reject(new Error(`timeout: ${selector}`))
      requestAnimationFrame(check)
    }
    check()
  })
}

const TOUR_STEPS: { sel: string; title: string; desc: string; tab: string }[] = [
  { sel: '[data-guide="dashboard"]', title: '总览 Dashboard', desc: '查看系统资源使用情况、统计数量和运行中实例列表', tab: 'dashboard' },
  { sel: '[data-guide="model-search"]', title: '模型仓库搜索', desc: '按模型名称、量化类型或架构名称过滤模型文件', tab: 'model-repo' },
  { sel: '[data-guide="download-source"]', title: '下载管理', desc: '选择 ModelScope 或 HuggingFace，输入仓库 ID 浏览并下载', tab: 'downloads' },
  { sel: '[data-guide="engine-scan"]', title: '引擎扫描', desc: '添加 llama-server 所在目录，点击扫描发现所有引擎', tab: 'engine' },
  { sel: '[data-guide="instance-create"]', title: '创建实例', desc: '点击创建，选择模型、引擎和端口，即可启动服务器', tab: 'instances' },
  { sel: '[data-guide="instance-config"]', title: '参数配置', desc: '点击实例卡片上的 ⚙ 配置按钮进入参数配置页面，支持 159 个参数和智能校验', tab: 'instances' },
  { sel: '[data-guide="perf-select"]', title: '性能监控', desc: '选择运行中的实例查看 CPU/GPU/显存实时指标和推理性能', tab: 'perf' },
  { sel: '[data-guide="logs-clear"]', title: '服务器日志', desc: '选择实例查看实时输出，支持关键词高亮、暂停/跟随、清空', tab: 'logs' },
]

export default function GuidePage() {
  const { setActiveTab } = useAppStore()
  const [html, setHtml] = useState('')
  const [toc, setToc] = useState<{ id: string; title: string }[]>([])
  const contentRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    setToc(parseTOC(guideMd))
    setHtml(renderMD(guideMd))
  }, [])

  const handleTocClick = (id: string) => {
    const el = contentRef.current?.querySelector(`#${id}`)
    if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  const startTour = async () => {
    // Step through each page, highlighting one element at a time
    // Each step creates a fresh driver instance and waits for user to click "done"
    for (let i = 0; i < TOUR_STEPS.length; i++) {
      const step = TOUR_STEPS[i]
      setActiveTab(step.tab)

      // Wait for the target element to appear after page switch
      const el = await waitDOM(step.sel, 5000).catch(() => {})
      if (!el) continue // skip if element not found

      const isLast = i === TOUR_STEPS.length - 1

      // Config page needs an active instance to show content
      if (step.tab === 'config') {
        const s = useAppStore.getState()
        if (s.instances.length === 0) continue // skip if no instances
        s.setActiveConfigInstanceId(s.instances[0].id)
      }

      await new Promise<void>(resolve => {
        const d = driver({
          animate: true,
          showProgress: true,
          steps: [{
            element: el as HTMLElement,
            popover: {
              title: step.title,
              description: step.desc,
              doneBtnText: isLast ? '完成' : `下一步: ${TOUR_STEPS[i + 1]?.title || ''}`,
            },
          }],
          onDestroyed: () => { resolve() },
        })
        d.drive()
      })
    }
  }

  return (
    <div className="flex flex-1 overflow-hidden h-full">
      {/* Left TOC sidebar — sticky, independent scroll */}
      <nav className="w-52 shrink-0 h-full overflow-y-auto border-r border-gray-200 dark:border-gray-700 p-3 space-y-0.5 bg-gray-50 dark:bg-gray-800/50">
        <button onClick={startTour}
          className="w-full px-3 py-2 mb-3 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs font-medium transition-colors">
          ▶ 开始交互式引导
        </button>
        {toc.map(item => (
          <button key={item.id}
            onClick={() => handleTocClick(item.id)}
            className="w-full text-left px-2 py-1 text-xs text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded truncate transition-colors"
            title={item.title}>
            {item.title}
          </button>
        ))}
      </nav>
      {/* Right content area */}
      <div ref={contentRef}
        className="flex-1 overflow-y-auto p-6 guide-content text-sm text-gray-800 dark:text-gray-200"
        dangerouslySetInnerHTML={{ __html: html }} />
    </div>
  )
}
