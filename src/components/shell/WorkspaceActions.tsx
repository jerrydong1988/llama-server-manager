import { createContext, useContext, useMemo, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

const WorkspaceActionsContext = createContext<{
  floatingTarget: HTMLDivElement | null
  guideTarget: HTMLDivElement | null
  setFloatingTarget: (target: HTMLDivElement | null) => void
  setGuideTarget: (target: HTMLDivElement | null) => void
} | null>(null)

// Pages retain ownership of their actions. The shell supplies destinations that
// cannot overlap its guide or footer, without measuring either one's height.
export function WorkspaceActionsProvider({ children }: { children: ReactNode }) {
  const [floatingTarget, setFloatingTarget] = useState<HTMLDivElement | null>(null)
  const [guideTarget, setGuideTarget] = useState<HTMLDivElement | null>(null)
  const value = useMemo(() => ({ floatingTarget, guideTarget, setFloatingTarget, setGuideTarget }), [floatingTarget, guideTarget])
  return <WorkspaceActionsContext.Provider value={value}>{children}</WorkspaceActionsContext.Provider>
}

export function useWorkspaceActions() {
  const context = useContext(WorkspaceActionsContext)
  if (!context) throw new Error('Workspace actions require the app shell')
  return context
}

export function WorkspaceFloatingActionsHost() {
  const { setFloatingTarget } = useWorkspaceActions()
  return <div ref={setFloatingTarget} data-workspace-floating-actions className="pointer-events-none absolute bottom-4 right-4 z-30 max-w-[calc(100%-2rem)] sm:right-5" />
}

export function WorkspaceFloatingActions({ children }: { children: ReactNode }) {
  const { floatingTarget } = useWorkspaceActions()
  return floatingTarget ? createPortal(children, floatingTarget) : null
}

export function WorkspaceGuideActionsHost() {
  const { setGuideTarget } = useWorkspaceActions()
  return <div ref={setGuideTarget} data-guide-page-actions className="empty:hidden" />
}
