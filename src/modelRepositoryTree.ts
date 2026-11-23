import type { ModelInfo } from './store/types'
import { formatPathForDisplay, isPathWithinRoot, normalizePath, pathJoin } from './utils/path'

interface TreeStats {
  models: number
  mmproj: number
  imatrix: number
  size: number
}

export interface ModelTreeNode {
  name: string
  path: string
  isDir: boolean
  children?: Map<string, ModelTreeNode>
  orderedChildren?: ModelTreeNode[]
  model?: ModelInfo
  stats: TreeStats
}

const emptyTreeStats = (): TreeStats => ({ models: 0, mmproj: 0, imatrix: 0, size: 0 })

const modelStats = (model: ModelInfo): TreeStats => ({
  models: model.file_type === 'model' && !model.is_shard ? 1 : 0,
  mmproj: model.file_type === 'mmproj' ? 1 : 0,
  imatrix: model.file_type === 'imatrix' ? 1 : 0,
  size: model.size,
})

const finalizeTree = (node: ModelTreeNode): TreeStats => {
  if (!node.isDir) return node.stats
  const orderedChildren = [...(node.children?.values() || [])].sort((left, right) => {
    if (left.isDir !== right.isDir) return left.isDir ? -1 : 1
    return left.name.localeCompare(right.name)
  })
  node.orderedChildren = orderedChildren
  const stats = emptyTreeStats()
  for (const child of orderedChildren) {
    const childStats = finalizeTree(child)
    stats.models += childStats.models
    stats.mmproj += childStats.mmproj
    stats.imatrix += childStats.imatrix
    stats.size += childStats.size
  }
  node.stats = stats
  return stats
}

const buildTree = (rootDir: string, models: ModelInfo[]): ModelTreeNode => {
  const normalizedRoot = normalizePath(rootDir)
  const root: ModelTreeNode = { name: rootDir, path: normalizedRoot, isDir: true, children: new Map(), stats: emptyTreeStats() }

  for (const model of models) {
    const normalizedPath = normalizePath(model.path)
    if (!isPathWithinRoot(normalizedPath, normalizedRoot)) {
      continue
    }

    const relative = normalizedRoot === '.' ? normalizedPath : normalizedPath.slice(normalizedRoot.length).replace(/^\/+/, '')
    if (!relative) {
      continue
    }

    const parts = relative.split('/')
    let cursor = root

    for (let index = 0; index < parts.length; index += 1) {
      const part = parts[index]
      if (index === parts.length - 1) {
        cursor.children!.set(part, { name: part, path: model.path, isDir: false, model, stats: modelStats(model) })
      } else {
        if (!cursor.children!.has(part)) {
          cursor.children!.set(part, {
            name: part,
            path: pathJoin(cursor.path, part),
            isDir: true,
            children: new Map(),
            stats: emptyTreeStats(),
          })
        }
        cursor = cursor.children!.get(part)!
      }
    }
  }

  finalizeTree(root)
  return root
}

export function buildModelTrees(modelDirs: string[], models: ModelInfo[]) {
  return modelDirs.map(dir => buildTree(dir, models))
}

export function matchesModelNode(node: ModelTreeNode, query: string) {
  return [node.name, node.model?.name, node.model?.quant_type, node.model?.architecture, node.model?.file_type]
    .some(value => value && value.toLowerCase().includes(query))
    || (node.isDir && formatPathForDisplay(node.name).toLowerCase().includes(query))
}

interface ModelTreeRow {
  node: ModelTreeNode
  branches: boolean[]
  key: string
}

export function visibleModelRows(roots: ModelTreeNode[], query: string, collapsed: ReadonlySet<string>): ModelTreeRow[] {
  const matches = new Set<ModelTreeNode>()
  const collectMatches = (node: ModelTreeNode): boolean => {
    let found = matchesModelNode(node, query)
    for (const child of node.orderedChildren ?? []) {
      if (collectMatches(child)) found = true
    }
    if (found) matches.add(node)
    return found
  }
  if (query) roots.forEach(collectMatches)

  const rows: ModelTreeRow[] = []
  const visible = (node: ModelTreeNode) => !query || matches.has(node)
  const visit = (rootPath: string, node: ModelTreeNode, branches: boolean[]) => {
    // A file may also appear under a separately registered nested scan root.
    const key = rootPath + '\0' + node.path
    rows.push({ node, branches, key })
    if (!node.orderedChildren || collapsed.has(key)) return
    const children = query ? node.orderedChildren.filter(visible) : node.orderedChildren
    children.forEach((child, index) => visit(rootPath, child, [...branches, index < children.length - 1]))
  }
  for (const root of roots) {
    if (visible(root)) visit(root.path, root, [])
  }
  return rows
}
