import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { FolderOpen, Info } from 'lucide-react'
import { useEffect, useState } from 'react'

const apiConfigSchema = z.object({
  host: z.string().default('127.0.0.1'),
  port: z.number().int().min(1).max(65535).default(8080),
  apiKey: z.string().optional(),
  sslKeyFile: z.string().optional(),
  sslCertFile: z.string().optional(),
  noUi: z.boolean().default(false),
  embedding: z.boolean().default(false),
  pooling: z.string().optional(),
  reranking: z.boolean().default(false),
  verbose: z.boolean().default(false),
  customArgs: z.array(z.string()).default([]),
})

type APIConfig = z.infer<typeof apiConfigSchema>

const poolingOptions = [
  { value: '', label: '默认' },
  { value: 'none', label: 'None' },
  { value: 'mean', label: 'Mean (推荐)' },
  { value: 'cls', label: 'CLS' },
  { value: 'last', label: 'Last' },
  { value: 'rank', label: 'Rank' },
]

const APIConfig = () => {
  const [newArg, setNewArg] = useState('')
  const { register, handleSubmit, watch, setValue, formState: { errors } } = useForm<APIConfig>({
    resolver: zodResolver(apiConfigSchema),
    defaultValues: {
      host: '127.0.0.1',
      port: 8080,
      apiKey: '',
      sslKeyFile: '',
      sslCertFile: '',
      noUi: false,
      embedding: false,
      pooling: '',
      reranking: false,
      verbose: false,
      customArgs: [],
    }
  })

  const embeddingMode = watch('embedding')
  const customArgs = watch('customArgs') || []

  useEffect(() => {
    if (embeddingMode && !watch('pooling')) {
      setValue('pooling', 'mean')
    }
  }, [embeddingMode, setValue, watch])

  const onSubmit = (data: APIConfig) => {
    console.log('API配置', data)
  }

  const addCustomArg = () => {
    if (newArg.trim()) {
      setValue('customArgs', [...customArgs, newArg.trim()])
      setNewArg('')
    }
  }

  const removeCustomArg = (index: number) => {
    setValue('customArgs', customArgs.filter((_, i) => i !== index))
  }

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-8">
      {/* 网络配置 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">网络配置</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              监听地址 (host)
            </label>
            <input
              {...register('host')}
              type="text"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              placeholder="127.0.0.1 或 0.0.0.0 允许外部访问"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              0.0.0.0 允许局域网内其他设备访问
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              监听端口 (port)
            </label>
            <input
              {...register('port', { valueAsNumber: true })}
              type="number"
              min="1"
              max="65535"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              API 密钥
            </label>
            <input
              {...register('apiKey')}
              type="password"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              placeholder="设置后请求需要携带Authorization头"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              开启后可以防止未授权访问API
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              SSL 私钥文件
            </label>
            <div className="flex gap-2">
              <input
                {...register('sslKeyFile')}
                type="text"
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                placeholder=".key 文件路径"
              />
              <button
                type="button"
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors"
              >
                <FolderOpen className="w-5 h-5" />
              </button>
            </div>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              SSL 证书文件
            </label>
            <div className="flex gap-2">
              <input
                {...register('sslCertFile')}
                type="text"
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                placeholder=".pem/.crt 文件路径"
              />
              <button
                type="button"
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors"
              >
                <FolderOpen className="w-5 h-5" />
              </button>
            </div>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              配置后启用HTTPS协议
            </p>
          </div>
        </div>
      </div>

      {/* 功能配置 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">功能配置</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div className="flex items-start gap-2">
            <input
              {...register('noUi')}
              type="checkbox"
              id="noUi"
              className="w-4 h-4 mt-1 rounded border-gray-300"
            />
            <div>
              <label htmlFor="noUi" className="text-sm font-medium">
                禁用内置WebUI
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                新版llama-server默认内置WebUI，勾选后禁用
              </p>
            </div>
          </div>

          <div className="flex items-start gap-2">
            <input
              {...register('embedding')}
              type="checkbox"
              id="embedding"
              className="w-4 h-4 mt-1 rounded border-gray-300"
            />
            <div>
              <label htmlFor="embedding" className="text-sm font-medium">
                仅嵌入模式 (Embedding Only)
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                向量模型专用，禁用文本生成功能，加载向量模型时建议开启
              </p>
            </div>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              嵌入池化策略
            </label>
            <select
              {...register('pooling')}
              disabled={!embeddingMode}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 disabled:opacity-50"
            >
              {poolingOptions.map(opt => (
                <option key={opt.value} value={opt.value}>{opt.label}</option>
              ))}
            </select>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              Mean是通用推荐，BGE模型建议用CLS
            </p>
          </div>

          <div className="flex items-start gap-2">
            <input
              {...register('reranking')}
              type="checkbox"
              id="reranking"
              className="w-4 h-4 mt-1 rounded border-gray-300"
            />
            <div>
              <label htmlFor="reranking" className="text-sm font-medium">
                启用重排序端点
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                重排序模型专用，启用/v1/rerank端点
              </p>
            </div>
          </div>

          <div className="flex items-start gap-2">
            <input
              {...register('verbose')}
              type="checkbox"
              id="verbose"
              className="w-4 h-4 mt-1 rounded border-gray-300"
            />
            <div>
              <label htmlFor="verbose" className="text-sm font-medium">
                详细日志 (verbose)
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                输出更详细的调试日志，排查问题时开启
              </p>
            </div>
          </div>
        </div>
      </div>

      {/* 自定义参数 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">自定义参数</h3>
        <div className="space-y-4">
          <div className="flex gap-2">
            <input
              type="text"
              value={newArg}
              onChange={(e) => setNewArg(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && addCustomArg()}
              className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              placeholder="例如 --my-custom-arg value"
            />
            <button
              type="button"
              onClick={addCustomArg}
              className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"
            >
              添加
            </button>
          </div>

          {customArgs.length > 0 && (
            <div className="space-y-2">
              {customArgs.map((arg, index) => (
                <div key={index} className="flex items-center justify-between p-2 bg-gray-50 dark:bg-gray-700 rounded-lg">
                  <code className="text-sm">{arg}</code>
                  <button
                    type="button"
                    onClick={() => removeCustomArg(index)}
                    className="p-1 text-red-600 hover:bg-red-100 dark:hover:bg-red-900/30 rounded transition-colors"
                  >
                    删除
                  </button>
                </div>
              ))}
            </div>
          )}

          {customArgs.length === 0 && (
            <p className="text-sm text-gray-500 dark:text-gray-400 text-center py-4">
              暂无自定义参数，添加GUI未覆盖的额外参数
            </p>
          )}
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

export default APIConfig
