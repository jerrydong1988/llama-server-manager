import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { FolderOpen, Info } from 'lucide-react'

const modelConfigSchema = z.object({
  modelPath: z.string().min(1, '模型路径不能为空'),
  alias: z.string().optional(),
  loraPath: z.string().optional(),
  mmprojPath: z.string().optional(),
  grammarFile: z.string().optional(),
  chatTemplate: z.string().optional(),
  reasoningFormat: z.enum(['', 'auto', 'none', 'deepseek']).optional(),
  reasoningEffort: z.enum(['', 'low', 'medium', 'high']).optional(),
  reasoning: z.enum(['', 'on', 'off', 'auto']).optional(),
  jinja: z.boolean().default(false),
  reasoningBudget: z.number().int().min(0).optional(),
})

type ModelConfig = z.infer<typeof modelConfigSchema>

const ModelConfig = () => {
  const { register, handleSubmit, formState: { errors } } = useForm<ModelConfig>({
    resolver: zodResolver(modelConfigSchema),
    defaultValues: {
      modelPath: '',
      alias: '',
      loraPath: '',
      mmprojPath: '',
      grammarFile: '',
      chatTemplate: '',
      reasoningFormat: '',
      reasoningEffort: '',
      reasoning: '',
      jinja: false,
      reasoningBudget: 0,
    }
  })

  const onSubmit = (data: ModelConfig) => {
    console.log('模型配置', data)
  }

  const selectFile = async (field: keyof ModelConfig) => {
    // 后续对接Tauri文件选择对话框
  }

  const chatTemplates = [
    { value: '', label: '自动检测' },
    { value: 'bailing', label: 'Bailing' },
    { value: 'chatglm3', label: 'ChatGLM3' },
    { value: 'chatglm4', label: 'ChatGLM4' },
    { value: 'chatml', label: 'ChatML' },
    { value: 'command-r', label: 'Command-R' },
    { value: 'deepseek', label: 'DeepSeek' },
    { value: 'deepseek2', label: 'DeepSeek V2' },
    { value: 'deepseek3', label: 'DeepSeek V3' },
    { value: 'exaone3', label: 'EXAONE 3' },
    { value: 'gemma', label: 'Gemma' },
    { value: 'gpt-oss', label: 'GPT-OSS' },
    { value: 'kimi-k2', label: 'Kimi K2' },
    { value: 'llama2', label: 'Llama 2' },
    { value: 'llama3', label: 'Llama 3' },
    { value: 'llama4', label: 'Llama 4' },
    { value: 'mistral', label: 'Mistral' },
    { value: 'openchat', label: 'OpenChat' },
    { value: 'phi3', label: 'Phi 3' },
    { value: 'phi4', label: 'Phi 4' },
    { value: 'vicuna', label: 'Vicuna' },
    { value: 'zephyr', label: 'Zephyr' },
  ]

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-8">
      {/* 基础模型配置 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">基础模型配置</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              模型路径 <span className="text-red-500">*</span>
            </label>
            <div className="flex gap-2">
              <input
                {...register('modelPath')}
                type="text"
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                placeholder="选择GGUF模型文件"
              />
              <button
                type="button"
                onClick={() => selectFile('modelPath')}
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors"
              >
                <FolderOpen className="w-5 h-5" />
              </button>
            </div>
            {errors.modelPath && <p className="mt-1 text-sm text-red-500">{errors.modelPath.message}</p>}
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              模型别名
            </label>
            <input
              {...register('alias')}
              type="text"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              placeholder="API调用时使用的别名"
            />
          </div>
        </div>
      </div>

      {/* 模型扩展 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">模型扩展</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              LoRA 路径
            </label>
            <div className="flex gap-2">
              <input
                {...register('loraPath')}
                type="text"
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                placeholder="选择LoRA适配器文件"
              />
              <button
                type="button"
                onClick={() => selectFile('loraPath')}
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors"
              >
                <FolderOpen className="w-5 h-5" />
              </button>
            </div>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              多模态投影器
            </label>
            <div className="flex gap-2">
              <input
                {...register('mmprojPath')}
                type="text"
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                placeholder="选择多模态投影器文件"
              />
              <button
                type="button"
                onClick={() => selectFile('mmprojPath')}
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors"
              >
                <FolderOpen className="w-5 h-5" />
              </button>
            </div>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              语法文件
            </label>
            <div className="flex gap-2">
              <input
                {...register('grammarFile')}
                type="text"
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                placeholder="选择GBNF语法文件"
              />
              <button
                type="button"
                onClick={() => selectFile('grammarFile')}
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors"
              >
                <FolderOpen className="w-5 h-5" />
              </button>
            </div>
          </div>
        </div>
      </div>

      {/* 对话行为 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">对话行为</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              聊天模板
            </label>
            <select
              {...register('chatTemplate')}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            >
              {chatTemplates.map(tpl => (
                <option key={tpl.value} value={tpl.value}>{tpl.label}</option>
              ))}
            </select>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              推理格式
            </label>
            <select
              {...register('reasoningFormat')}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            >
              <option value="">自动</option>
              <option value="none">关闭</option>
              <option value="deepseek">DeepSeek</option>
            </select>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              推理力度
            </label>
            <select
              {...register('reasoningEffort')}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            >
              <option value="">默认</option>
              <option value="low">低</option>
              <option value="medium">中</option>
              <option value="high">高</option>
            </select>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              推理开关
            </label>
            <select
              {...register('reasoning')}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            >
              <option value="">自动</option>
              <option value="on">开启</option>
              <option value="off">关闭</option>
            </select>
          </div>

          <div className="flex items-center gap-2">
            <input
              {...register('jinja')}
              type="checkbox"
              id="jinja"
              className="w-4 h-4 rounded border-gray-300"
            />
            <label htmlFor="jinja" className="text-sm font-medium">
              启用 Jinja 模板
            </label>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              推理预算
            </label>
            <input
              {...register('reasoningBudget', { valueAsNumber: true })}
              type="number"
              min="0"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              placeholder="0 = 无限制"
            />
          </div>
        </div>
      </div>

      <div className="flex justify-end">
        <button
          type="submit"
          className="px-6 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"
        >
          保存配置
        </button>
      </div>
    </form>
  )
}

export default ModelConfig
