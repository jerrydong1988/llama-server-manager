import { ArrowLeft } from 'lucide-react'
import { useI18n } from '../../i18n'
import { useAppStore } from '../../store'
import { Button } from '../ui'

export function BackToInstancesButton({ disabled, compact }: { disabled?: boolean; compact?: boolean }) {
  const { t } = useI18n()
  const setActiveTab = useAppStore(state => state.setActiveTab)
  return (
    <Button variant="secondary" size={compact ? 'sm' : undefined} disabled={disabled}
      onClick={() => setActiveTab('instances')} icon={<ArrowLeft className="h-4 w-4" />}>
      {t.configPage.backToInstances}
    </Button>
  )
}
