export function getRoutingWorkspaceLabels(lang: string) {
  const zh = lang === 'zh-CN'
  return {
    routes: zh ? '路由与实例' : 'Routes & instances',
    settings: zh ? '路由设置' : 'Router settings',
    service: zh ? '路由服务' : 'Router service',
    targets: zh ? '目标实例' : 'Target instances',
    routeHelp: zh ? '路由匹配说明' : 'How routing works',
    routeEditor: zh ? '编辑路由' : 'Edit route',
    edit: zh ? '编辑' : 'Edit',
    done: zh ? '完成' : 'Done',
    unnamed: zh ? '未命名' : 'Unnamed',
    listen: zh ? '监听与 API 入口' : 'Listener & API endpoints',
    scheduling: zh ? '模型调度' : 'Model scheduling',
    traffic: zh ? '流量治理与健康检查' : 'Traffic & health checks',
    origins: zh ? '浏览器来源（CORS）' : 'Browser origins (CORS)',
    noOrigins: zh ? '未添加来源' : 'No origins configured',
    keys: zh ? '调用方密钥' : 'Caller API keys',
    keyHint: zh
      ? '拥有推理权限的 Key 可调用所有已发布路由。'
      : 'Keys with inference permission can call all published routes.',
    allDisabled: zh
      ? '所有 Key 均已停用，受 Key 保护的调用将被拒绝。'
      : 'All keys are disabled. Requests that require a key will be rejected.',
    legacyKey: zh
      ? '当前使用旧版调用凭据。保存配置时会迁移为命名 Key。'
      : 'A legacy credential is in use. Saving the configuration migrates it to a named key.',
    all: zh ? '全部' : 'All',
    disabled: zh ? '已停用' : 'Disabled',
    search: zh ? '搜索 Key 名称' : 'Search key names',
    noMatches: zh ? '没有符合条件的 Key' : 'No matching keys',
    emptyKeys: zh ? '尚无具名 API Key' : 'No named API keys',
    clear: zh ? '清除筛选' : 'Clear filters',
    rate: zh ? '请求 / 分钟' : 'Requests / min',
    concurrency: zh ? '并发上限' : 'Concurrency',
    inherited: zh ? '沿用全局' : 'Global default',
    toggleKey: zh ? '启用此 Key' : 'Enable this key',
    previous: zh ? '上一页' : 'Previous page',
    next: zh ? '下一页' : 'Next page',
    page: (n: number) => (zh ? `第 ${n} 页` : `Page ${n}`),
    range: (first: number, last: number, total: number) =>
      zh ? `${first}–${last} 项，共 ${total} 项` : `${first}–${last} of ${total}`,
    editKey: zh ? '编辑 API Key' : 'Edit API key',
    general: zh ? '常规' : 'General',
    quotas: zh ? 'Token 额度' : 'Token quotas',
    credential: zh ? '密钥' : 'Credential',
    rotate: zh ? '轮换密钥' : 'Rotate key',
    copyBeforeSave: zh
      ? '请在保存前复制新密钥。保存后仅保留哈希。'
      : 'Copy the new credential before saving. Only its hash is retained.',
    rotateHint: zh ? '保存后旧密钥失效。' : 'Saving revokes the previous credential.',
    limitsHint: zh ? '填 0 时沿用全局设置。' : 'Use 0 to inherit global limits.',
    softHint: zh ? '达到软预算时提醒，填 0 关闭提醒。' : 'Soft budgets trigger reminders. Use 0 to turn reminders off.',
    saveKey: zh ? '保存此 Key' : 'Save this key',
    savedKey: zh ? '此 Key 已保存并生效' : 'Key saved and applied',
    removedKey: zh ? '已删除此 Key' : 'Key deleted',
    discardPrompt: zh ? '放弃此 Key 未保存的修改？' : 'Discard unsaved changes to this key?',
    discard: zh ? '放弃修改' : 'Discard changes',
    reloadPrompt: zh
      ? '重新加载会放弃尚未保存的路由和全局设置。'
      : 'Reloading discards unsaved route and global settings changes.',
    keepEditing: zh ? '继续编辑' : 'Keep editing',
    confirmRemove: zh ? '确认删除此 Key' : 'Confirm key deletion',
    removeLast: zh
      ? '删除最后一个 Key 后，本机路由将允许免凭据调用。'
      : 'Deleting the last key allows local requests without credentials.',
    removeHint: zh ? '删除后，使用此 Key 的客户端将失去访问权限。' : 'Clients using this credential will lose access.',
    keyConflict: zh
      ? '此 Key 已被其他操作修改。请刷新配置并重新打开此 Key。'
      : 'This key changed elsewhere. Refresh and reopen it before saving.',
    nameRequired: zh ? '请填写 Key 名称。' : 'Enter a key name.',
    credentialInvalid: zh ? '请输入新的密钥原文。' : 'Enter a new plaintext credential.',
    credentialLength: zh ? '新密钥至少需要 16 个字符。' : 'A new credential needs at least 16 characters.',
    replaceCredential: zh ? '可以在这里为调用方设置新密钥。' : 'Set a new credential for this caller here.',
    clipboardError: zh
      ? '无法复制，请临时显示并手动复制。'
      : 'Copy failed. Reveal the credential and copy it manually.',
  }
}
