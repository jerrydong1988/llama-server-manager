import { useState, useEffect } from 'react'
import { ChevronDown, ChevronRight, Settings, File, Image, X, FolderOpen } from 'lucide-react'
import { useAppStore, type InstanceConfig } from '../store'

const cacheTypes = ['', 'f32', 'f16', 'bf16', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1']
const specTypes = ['', 'none', 'draft-mtp', 'draft-simple', 'draft-eagle3', 'ngram-cache', 'ngram-simple']
const chatTemplates = ['', 'bailing', 'chatglm3', 'chatglm4', 'chatml', 'command-r', 'deepseek', 'deepseek2', 'deepseek3', 'exaone3', 'gemma', 'gpt-oss', 'kimi-k2', 'llama2', 'llama3', 'llama4', 'mistral', 'openchat', 'phi3', 'phi4', 'vicuna', 'zephyr']

const Section = ({ title, children, disabled, onToggle, toggled }: { title: string; children: React.ReactNode; disabled?: boolean; onToggle?: (v: boolean) => void; toggled?: boolean }) => {
  const [open, setOpen] = useState(false)
  return (
    <div className="border dark:border-gray-700 rounded-lg overflow-hidden">
      <button onClick={() => setOpen(!open)} className="w-full flex items-center gap-2 px-4 py-2.5 bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors text-left dark:text-gray-200">
        {open ? <ChevronDown className="w-4 h-4 text-gray-400 shrink-0" /> : <ChevronRight className="w-4 h-4 text-gray-400 shrink-0" />}
        <span className="text-sm font-medium">{title}</span>
        {disabled && <span className="text-xs text-gray-400 ml-1">🛑</span>}
        {onToggle !== undefined && (
          <label className="ml-auto flex items-center gap-1 cursor-pointer shrink-0" onClick={e => e.stopPropagation()} title={toggled ? '关闭高级采样' : '开启高级采样'}>
            <span className="text-xs text-gray-400">{toggled ? '已开启' : '已关闭'}</span>
            <input type="checkbox" checked={toggled} onChange={e => { e.stopPropagation(); onToggle(e.target.checked) }} className="w-3.5 h-3.5 rounded" />
          </label>
        )}
      </button>
      {open && <div className={`px-4 py-3 space-y-3 ${disabled ? 'opacity-50 pointer-events-none' : ''}`}>{children}</div>}
    </div>
  )
}

const Input = ({ label, value, onChange, placeholder, type, title, disabled }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string; type?: string; title?: string; disabled?: boolean }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <input type={type || 'text'} value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`} />
  </div>
)

const Num = ({ label, value, onChange, min, max, step, title, disabled }: { label: string; value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; title?: string; disabled?: boolean }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <input type="number" value={value} min={min} max={max} step={step || 1} onChange={e => onChange(parseFloat(e.target.value) || 0)} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`} />
  </div>
)

const Toggle = ({ label, value, onChange, title, disabled }: { label: string; value: boolean; onChange: (v: boolean) => void; title?: string; disabled?: boolean }) => (
  <label className={`flex items-center gap-2 ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`} title={title}>
    <input type="checkbox" checked={value} onChange={e => onChange(e.target.checked)} disabled={disabled} className="w-4 h-4 rounded" />
    <span className="text-sm">{label}</span>
  </label>
)

const Select = ({ label, value, onChange, options, title, disabled }: { label: string; value: string; onChange: (v: string) => void; options: string[]; title?: string; disabled?: boolean }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <select value={value} onChange={e => onChange(e.target.value)} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}>
      {options.map(o => <option key={o} value={o}>{o || '默认'}</option>)}
    </select>
  </div>
)

const ConfigPage = () => {
  const { instances, activeConfigInstanceId, updateInstance, saveConfig, models, modelDirs } = useAppStore()
  const inst = instances.find(i => i.id === activeConfigInstanceId)
  const [local, setLocal] = useState<InstanceConfig | null>(null)
  const [saved, setSaved] = useState(false)
  const [showPicker, setShowPicker] = useState(false)
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const [useAdvSampling, setUseAdvSampling] = useState(false)

  useEffect(() => { if (inst) setLocal({ ...inst.config }); else setLocal(null) }, [activeConfigInstanceId, instances])

  // 检测向量模型
  const EMBED_ARCHS = ['bge', 'gte', 'e5', 'text-embedding', 'sentence-bert', 'sentence-t5', 'instructor', 'bert', 'nomic', 'jina']
  const isEmbedding = (() => {
    if (!local?.model_path) return false
    const fname = local.model_path.replace(/\\/g, '/').split('/').pop() || ''
    if (fname.toLowerCase().includes('embed')) return true
    const model = models.find(m => m.path === local.model_path)
    if (model?.architecture && EMBED_ARCHS.some(a => model.architecture!.toLowerCase().includes(a))) return true
    return false
  })()

  // 检测到向量模型自动设置 embedding + pooling
  useEffect(() => {
    if (isEmbedding && local) {
      if (!local.embedding) set('embedding', true)
      if (!local.pooling) set('pooling', 'mean')
    }
  }, [isEmbedding, local?.model_path])

  if (!local) return (
    <div className="bg-white dark:bg-gray-800 rounded-lg p-12 text-center border dark:border-gray-700">
      <Settings className="w-12 h-12 mx-auto mb-3 text-gray-400" />
      <p className="text-gray-500">请先在「实例管理」中点击实例的 ⚙️ 按钮，选择要配置的实例</p>
    </div>
  )

  const set = (k: keyof InstanceConfig, v: any) => setLocal(l => l ? { ...l, [k]: v } : l)

  const pickModel = (modelPath: string) => {
    set('model_path', modelPath)
    const dir = modelPath.replace(/[/\\][^/\\]*$/, '')
    const mmproj = models.find(m => { const mDir = m.path.replace(/[/\\][^/\\]*$/, ''); return m.file_type === 'mmproj' && mDir === dir })
    if (mmproj) set('mmproj_path', mmproj.path); else set('mmproj_path', '')
    setShowPicker(false)
  }

  const save = () => { if (!local || !inst) return; updateInstance(inst.id, { config: local }); saveConfig(); setSaved(true); setTimeout(() => setSaved(false), 2000) }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Settings className="w-5 h-5 text-blue-500" />
          <span className="text-lg font-semibold">实例配置</span>
          <span className="text-sm text-gray-500">— {inst?.name}</span>
        </div>
        <button onClick={save} className="px-6 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors">{saved ? '✓ 已保存' : '保存配置'}</button>
      </div>
      {saved && <div className="bg-green-50 dark:bg-green-900/20 rounded-lg p-3 text-sm text-green-600 dark:text-green-400">配置已保存到「{inst?.name}」。回到实例管理后点击启动即可使用新配置。</div>}
      {isEmbedding && <div className="bg-blue-50 dark:bg-blue-900/20 rounded-lg p-3 text-sm text-blue-600 dark:text-blue-400">📊 检测到向量模型 — 已自动启用 Embedding 模式。采样/推测解码/对话行为等生成参数已禁用，仅向量模型相关的参数可配置。</div>}

      <div className="space-y-3">

        <Section title="基本参数">
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <div title="GGUF 模型文件的路径">
              <label className="block text-xs font-medium mb-1 text-gray-500">模型路径</label>
              <div className="flex gap-1">
                <input type="text" value={local.model_path} onChange={e => set('model_path', e.target.value)} className="flex-1 px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                <button onClick={() => setShowPicker(true)} className="px-2 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs" title="从模型仓库中选择">📂</button>
              </div>
            </div>
            <Input label="别名" value={local.alias} onChange={v => set('alias', v)} title="为模型设置别名（API 调用时使用）" />
            <Input label="LoRA 路径" value={local.lora_path} onChange={v => set('lora_path', v)} title="LoRA 适配器文件的路径（可选）" disabled={isEmbedding} />
            <Input label="多模态投影器" value={local.mmproj_path} onChange={v => set('mmproj_path', v)} title="多模态投影器文件的路径（视觉模型用）" disabled={isEmbedding} />
            <Input label="语法文件" value={local.grammar_file} onChange={v => set('grammar_file', v)} title="结构化输出用的 GBNF 语法文件路径（*.gbnf）" disabled={isEmbedding} />
            <Select label="聊天模板" value={local.chat_template} onChange={v => set('chat_template', v)} options={chatTemplates} title="选择聊天模板（留空自动检测）" disabled={isEmbedding} />
            <Select label="推理格式" value={local.reasoning_format} onChange={v => set('reasoning_format', v)} options={['', 'auto', 'none', 'deepseek']} title="控制是否允许/提取回复中的思考标签" disabled={isEmbedding} />
            <Select label="推理开关" value={local.reasoning} onChange={v => set('reasoning', v)} options={['', 'on', 'off', 'auto']} title="启用/禁用/自动推理（思考）功能。off 时加载更快" disabled={isEmbedding} />
            <Toggle label="Jinja 模板" value={local.jinja} onChange={v => set('jinja', v)} title="启用 Jinja2 模板（某些自定义模板需要）" disabled={isEmbedding} />
            <Select label="推理力度" value={local.reasoning_effort} onChange={v => set('reasoning_effort', v)} options={['', 'low', 'medium', 'high']} title="为聊天模板设置推理力度（部分模型支持）" disabled={isEmbedding} />
            <Num label="推理预算" value={local.reasoning_budget ? parseInt(local.reasoning_budget) : 0} onChange={v => set('reasoning_budget', v.toString())} min={0} max={65536} step={256} title="推理（思考）过程的令牌预算（0 = 无限制）" disabled={isEmbedding} />
          </div>
        </Section>

        <Section title="生成参数" disabled={isEmbedding}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label="生成令牌数" value={local.n_predict} onChange={v => set('n_predict', v)} min={-1} title="生成的令牌数（-1 = 无限）" />
            <Toggle label="忽略结束标记" value={local.ignore_eos} onChange={v => set('ignore_eos', v)} title="防止模型提前停止" />
            <Input label="JSON Schema" value={local.json_schema} onChange={v => set('json_schema', v)} title="JSON Schema 约束，限制输出为合法 JSON 格式" />
            <Num label="温度" value={local.temp} onChange={v => set('temp', v)} min={0} max={2} step={0.1} title="创造力级别。越低越确定，越高越有创造力" />
            <Num label="Top-K" value={local.top_k} onChange={v => set('top_k', v)} min={0} title="采样时仅保留 top-k 个令牌" />
            <Num label="Top-P" value={local.top_p} onChange={v => set('top_p', v)} min={0} max={1} step={0.1} title="核采样" />
            <Num label="重复惩罚" value={local.repeat_penalty} onChange={v => set('repeat_penalty', v)} min={0} max={2} step={0.1} title="增加以减少重复循环" />
            <Num label="随机种子" value={local.seed} onChange={v => set('seed', v)} min={-1} title="RNG 种子（-1 = 随机）。设为固定值可重现结果" />
            <Num label="Min-P" value={local.min_p} onChange={v => set('min_p', v)} min={0} max={1} step={0.05} title="最小概率采样。比 top-p 更新的采样方式" />
            <Num label="存在惩罚" value={local.presence_penalty} onChange={v => set('presence_penalty', v)} min={0} max={2} step={0.1} title="话题存在惩罚。降低重复讨论相同话题" />
            <Num label="频率惩罚" value={local.frequency_penalty} onChange={v => set('frequency_penalty', v)} min={0} max={2} step={0.1} title="词频惩罚。降低高频词重复出现" />
            <Num label="惩罚窗口" value={local.repeat_last_n} onChange={v => set('repeat_last_n', v)} min={-1} title="重复惩罚考虑的最近令牌数（-1 = 上下文大小）" />
          </div>
        </Section>

        <Section title="高级采样" disabled={isEmbedding || !useAdvSampling} onToggle={v => { setUseAdvSampling(v); if (!v) { set('mirostat', 0); set('mirostat_lr', 0); set('mirostat_ent', 0); set('xtc_probability', 0); set('xtc_threshold', 0); set('dynatemp_range', 0); set('dynatemp_exp', 0); set('typical_p', 1); set('dry_multiplier', 0); set('dry_base', 0); set('dry_allowed_length', 0); set('dry_penalty_last_n', 0); set('dry_sequence_breaker', '') } }} toggled={useAdvSampling}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label="Mirostat 模式" value={local.mirostat.toString()} onChange={v => set('mirostat', parseInt(v))} options={['0', '1', '2']} title="Mirostat 自适应采样。0=禁用，1=v1，2=v2（推荐）" />
            <Num label="Mirostat 学习率" value={local.mirostat_lr} onChange={v => set('mirostat_lr', v)} min={0.001} max={1} step={0.001} title="Mirostat 学习率。推荐 0.05~0.1，创作用 0.01" />
            <Num label="Mirostat 目标熵" value={local.mirostat_ent} onChange={v => set('mirostat_ent', v)} min={0} max={10} step={0.1} title="Mirostat 目标熵。越高越随机。对话 4.0，故事 5.0" />
            <Num label="XTC 概率" value={local.xtc_probability} onChange={v => set('xtc_probability', v)} min={0} max={1} step={0.05} title="XTC 采样概率。以该概率排除太明显的词。推荐 0.1~0.3。0=禁用" />
            <Num label="XTC 阈值" value={local.xtc_threshold} onChange={v => set('xtc_threshold', v)} min={0} max={1} step={0.05} title="XTC 阈值。词的概率超过此值即被排除。推荐 0.1~0.5" />
            <Num label="动态温度范围" value={local.dynatemp_range} onChange={v => set('dynatemp_range', v)} min={0} max={10} step={0.1} title="动态温度范围。实际温度在 [temp-range, temp+range] 间摇摆。0=禁用" />
            <Num label="动态温度指数" value={local.dynatemp_exp} onChange={v => set('dynatemp_exp', v)} min={0} max={10} step={0.1} title="动态温度指数。调节对概率分布宽度的敏感度。推荐 1.0" />
            <Num label="Typical-P" value={local.typical_p} onChange={v => set('typical_p', v)} min={0} max={1} step={0.05} title="局部典型采样。比 Top-P 更自然的选词策略。推荐 0.9~0.95。1.0=禁用" />
            <Num label="DRY 惩罚强度" value={local.dry_multiplier} onChange={v => set('dry_multiplier', v)} min={0} max={10} step={0.1} title="DRY 重复惩罚强度。检测重复短语/句式并降权。推荐 0.8~1.2。0=禁用" />
            <Num label="DRY 基数" value={local.dry_base} onChange={v => set('dry_base', v)} min={0} max={10} step={0.1} title="DRY 惩罚增长曲线基数。通常保持默认 1.75" />
            <Num label="DRY 允许长度" value={local.dry_allowed_length} onChange={v => set('dry_allowed_length', v)} min={0} step={1} title="DRY 允许重复的连续令牌数。推荐 2~3。默认 2" />
            <Num label="DRY 惩罚窗口" value={local.dry_penalty_last_n} onChange={v => set('dry_penalty_last_n', v)} min={-1} title="DRY 扫描多少最近令牌检测重复。-1=整个上下文" />
            <Input label="DRY 分隔符" value={local.dry_sequence_breaker} onChange={v => set('dry_sequence_breaker', v)} title="DRY 序列分隔符。写入后遇到此字符视为打断重复" />
          </div>
        </Section>

        <Section title="性能配置">
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Toggle label="自动上下文" value={local.ctx_size_auto} onChange={v => set('ctx_size_auto', v)} title="勾选后不传 -c 参数，llama-server 自动使用模型完整上下文长度" />
            {!local.ctx_size_auto && <Num label="上下文大小" value={local.ctx_size} onChange={v => set('ctx_size', v)} min={0} step={1024} title="模型的上下文大小（序列长度）" />}
            <Num label="GPU 层数" value={local.gpu_layers} onChange={v => set('gpu_layers', v)} min={0} max={99} title="卸载到 GPU 的模型层数（99 = 全部）" />
            <Num label="线程数" value={local.threads} onChange={v => set('threads', v)} min={0} title="使用的 CPU 线程数。留空=自动" />
            <Num label="批大小" value={local.batch_size} onChange={v => set('batch_size', v)} min={1} title="提示处理的批大小" />
            <Num label="物理批大小" value={local.ubatch_size} onChange={v => set('ubatch_size', v)} min={1} title="物理批大小。较低值减少显存占用但降低速度" />
            <Num label="并行序列" value={local.parallel} onChange={v => set('parallel', v)} min={1} title="并行处理的序列数" />
            <Toggle label="持续批处理" value={local.cont_batching} onChange={v => set('cont_batching', v)} title="启用持续批处理以提高吞吐量" />
            <Toggle label="提示缓存" value={local.cache_prompt} onChange={v => set('cache_prompt', v)} title="启用提示缓存以提高重复请求的速度" />
            <Num label="批处理线程" value={local.threads_batch} onChange={v => set('threads_batch', v)} min={0} title="提示处理和批处理时使用的线程数。留空=默认同 -t" />
          </div>
        </Section>

        <Section title="高级">
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label="Flash Attn" value={local.flash_attn} onChange={v => set('flash_attn', v)} options={['auto', 'on', 'off']} title="设置 Flash Attention（on/off/auto）" />
            <Num label="MoE CPU 层数" value={local.moe_cpu_layers} onChange={v => set('moe_cpu_layers', v)} min={0} max={99} title="GPU 放不下时保留在 CPU 上的 MoE 层数" disabled={isEmbedding} />
            <Toggle label="内存锁定" value={local.mlock} onChange={v => set('mlock', v)} title="将模型锁定在 RAM 中防止交换" />
            <Toggle label="禁用 mmap" value={local.no_mmap} onChange={v => set('no_mmap', v)} title="禁用模型文件的内存映射" />
            <Toggle label="NUMA 优化" value={local.numa} onChange={v => set('numa', v)} title="启用 NUMA 感知优化。仅多路 CPU 服务器需要" />
            <Select label="K 缓存类型" value={local.cache_type_k} onChange={v => set('cache_type_k', v)} options={cacheTypes} title="K 的 KV 缓存数据类型" />
            <Select label="V 缓存类型" value={local.cache_type_v} onChange={v => set('cache_type_v', v)} options={cacheTypes} title="V 的 KV 缓存数据类型" />
          </div>
          <div className="mt-3 grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label="草稿模型" value={local.draft_model_path} onChange={v => set('draft_model_path', v)} title="推测解码用的草稿模型路径" disabled={isEmbedding} />
            <Num label="草稿 GPU 层" value={local.draft_gpu_layers} onChange={v => set('draft_gpu_layers', v)} min={0} max={99} title="草稿模型的 GPU 层数" disabled={isEmbedding} />
            <Num label="草稿令牌数" value={local.draft_tokens} onChange={v => set('draft_tokens', v)} min={0} title="推测解码最大草稿令牌数" disabled={isEmbedding} />
            <Num label="最小草稿令牌" value={local.spec_draft_n_min} onChange={v => set('spec_draft_n_min', v)} min={0} title="推测解码最小草稿令牌数" disabled={isEmbedding} />
            <Select label="推测解码类型" value={local.spec_type} onChange={v => set('spec_type', v)} options={specTypes} title="推测解码类型。多个可用逗号分隔" disabled={isEmbedding} />
            <Num label="超时(秒)" value={local.timeout} onChange={v => set('timeout', v)} min={1} title="服务器读写超时秒数" />
            <Num label="空闲休眠(秒)" value={local.sleep_idle} onChange={v => set('sleep_idle', v)} min={-1} title="空闲 N 秒后自动卸载模型释放显存（-1 = 禁用）" />
            <Toggle label="上下文偏移" value={local.context_shift} onChange={v => set('context_shift', v)} title="无限生成时的上下文偏移策略，避免超出上下文窗口" disabled={isEmbedding} />
          </div>
        </Section>

        <Section title="网络 & API">
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label="主机" value={local.host} onChange={v => set('host', v)} title="监听的 IP 地址（0.0.0.0 允许网络访问）" />
            <Num label="端口" value={local.port} onChange={v => set('port', v)} min={1} max={65535} title="服务器监听的网络端口" />
            <Input label="API 密钥" value={local.api_key} onChange={v => set('api_key', v)} type="password" title="API 密钥，用于令牌认证（可选）" />
            <Toggle label="禁用 WebUI" value={local.no_ui} onChange={v => set('no_ui', v)} title="勾选后禁用内置 WebUI" />
            <Toggle label="Embedding 模式" value={local.embedding} onChange={v => set('embedding', v)} title="启用仅嵌入模式（禁用聊天和生成功能）" />
            <Select label="池化" value={local.pooling} onChange={v => set('pooling', v)} options={['', 'none', 'mean', 'cls', 'last', 'rank']} title="Embedding 模型的池化策略。mean=全局平均（通用推荐）" />
            <Toggle label="重排序" value={local.reranking} onChange={v => set('reranking', v)} title="启用 /v1/rerank 端点（RAG 检索重排序）" />
            <Toggle label="详细日志" value={local.verbose} onChange={v => set('verbose', v)} title="启用详细服务器日志以便调试" />
          </div>
          <div className="mt-3 grid grid-cols-2 gap-3">
            <Input label="SSL 私钥" value={local.ssl_key_file} onChange={v => set('ssl_key_file', v)} title="SSL 私钥文件路径（启用 HTTPS）" />
            <Input label="SSL 证书" value={local.ssl_cert_file} onChange={v => set('ssl_cert_file', v)} title="SSL 证书文件路径" />
          </div>
        </Section>

      </div>

      {showPicker && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg w-full max-w-2xl max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between px-6 py-4 border-b dark:border-gray-700">
              <h3 className="text-lg font-semibold">从模型仓库选择</h3>
              <button onClick={() => setShowPicker(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-5 h-5" /></button>
            </div>
            <div className="flex-1 overflow-y-auto p-4">
              {(function TreeRenderer() {
                interface TNode { name: string; path: string; isDir: boolean; children?: Map<string, TNode>; model?: typeof models[0] }
                function buildTree(rootDir: string): TNode {
                  const root: TNode = { name: rootDir, path: rootDir, isDir: true, children: new Map() }
                  const normRoot = rootDir.replace(/\\/g, '\\').toLowerCase()
                  for (const m of models) {
                    const normPath = m.path.replace(/\\/g, '\\').toLowerCase()
                    if (!normPath.startsWith(normRoot)) continue
                    const rel = m.path.substring(rootDir.length).replace(/^[\\/]+/, '')
                    if (!rel) continue
                    const parts = rel.split(/[\\/]/)
                    let cur = root
                    for (let i = 0; i < parts.length; i++) {
                      if (i === parts.length - 1) { cur.children!.set(parts[i], { name: parts[i], path: m.path, isDir: false, model: m }) }
                      else {
                        if (!cur.children!.has(parts[i])) {
                          cur.children!.set(parts[i], { name: parts[i], path: cur.path + (cur.path.endsWith('\\') ? '' : '\\') + parts[i], isDir: true, children: new Map() })
                        }
                        cur = cur.children!.get(parts[i])!
                      }
                    }
                  }
                  return root
                }
                const toggleP = (k: string) => { const n = new Set(pickerCollapsed); if (n.has(k)) n.delete(k); else n.add(k); setPickerCollapsed(n) }
                function renderNode(node: TNode, depth: number): any {
                  if (node.isDir) {
                    const c = pickerCollapsed.has(node.path)
                    return (
                      <div key={node.path}>
                        <button onClick={() => toggleP(node.path)} style={{ paddingLeft: `${depth * 12 + 4}px` }} className="w-full flex items-center gap-1.5 py-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-left text-xs">
                          {c ? <ChevronRight className="w-3 h-3 text-gray-400 shrink-0" /> : <ChevronDown className="w-3 h-3 text-gray-400 shrink-0" />}
                          {depth === 0 ? <FolderOpen className="w-3 h-3 text-yellow-500 shrink-0" /> : <span className="text-xs shrink-0">📁</span>}
                          <span className="truncate font-medium text-xs">{node.name}</span>
                        </button>
                        {!c && node.children && [...node.children.values()].sort((a, b) => { if (a.isDir !== b.isDir) return a.isDir ? -1 : 1; return a.name.localeCompare(b.name) }).map(ch => renderNode(ch, depth + 1))}
                      </div>
                    )
                  }
                  const m = node.model!
                  if (m.file_type === 'mmproj') return (
                    <div key={node.path} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="flex items-center gap-2 py-1 pr-2 text-xs text-gray-500">
                      <Image className="w-3 h-3 text-purple-500 shrink-0" />
                      <span className="truncate flex-1">{m.name}</span>
                      <span className="text-purple-400 shrink-0 text-xs">投影器</span>
                    </div>
                  )
                  return (
                    <button key={node.path} onClick={() => pickModel(m.path)} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="w-full flex items-center gap-2 py-1 pr-2 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded text-left text-xs">
                      <File className="w-3 h-3 text-blue-500 shrink-0" />
                      <span className="truncate flex-1">{m.name}</span>
                      <span className="text-gray-400 shrink-0">{m.quant_type || ''}</span>
                      <span className="text-gray-400 shrink-0">{m.size > 1024 * 1024 * 1024 ? (m.size / 1024 / 1024 / 1024).toFixed(1) + ' GB' : m.size > 1024 * 1024 ? (m.size / 1024 / 1024).toFixed(1) + ' MB' : m.size > 1024 ? (m.size / 1024).toFixed(1) + ' KB' : m.size + ' B'}</span>
                    </button>
                  )
                }
                return modelDirs.map(d => buildTree(d)).map(t => renderNode(t, 0))
              })()}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default ConfigPage
