import type { Lang } from './index'

export function getAgentManagementLabels(lang: Lang) {
  return {
    routerConflict: lang === 'zh-CN'
      ? '路由已被外部管理客户端修改，当前编辑已保留。请重新加载后合并修改。'
      : 'An external client changed the router. Your edits were preserved; reload and reconcile them before saving.',
  }
}
