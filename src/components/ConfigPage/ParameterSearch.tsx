import { useEffect, useState, type KeyboardEvent } from 'react'
import { ChevronDown, ChevronUp, Search, X } from 'lucide-react'
import type { getConfigPageLabels } from '../../i18n/configPageCopy'
import { Button, Surface, TextInput } from '../ui'

type Labels = ReturnType<typeof getConfigPageLabels>

const matchedFields = () => Array.from(
  document.querySelectorAll<HTMLElement>('[data-config-search-match="true"]'),
)

const clearCurrentResult = () => {
  document.querySelectorAll<HTMLElement>('[data-config-search-current="true"]')
    .forEach(element => element.removeAttribute('data-config-search-current'))
}

const focusResult = (element: HTMLElement) => {
  clearCurrentResult()
  element.dataset.configSearchCurrent = 'true'
  element.scrollIntoView({ behavior: 'smooth', block: 'center' })
  requestAnimationFrame(() => {
    element.querySelector<HTMLElement>('input, select, textarea, button')?.focus({ preventScroll: true })
  })
}

export function ParameterSearch({ query, onQueryChange, labels }: { query: string; onQueryChange: (value: string) => void; labels: Labels }) {
  const [matchCount, setMatchCount] = useState(0)
  const [activeIndex, setActiveIndex] = useState(-1)

  useEffect(() => {
    clearCurrentResult()
    setActiveIndex(-1)
    if (!query.trim()) {
      setMatchCount(0)
      return
    }
    let secondFrame = 0
    const firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => setMatchCount(matchedFields().length))
    })
    return () => {
      cancelAnimationFrame(firstFrame)
      if (secondFrame) cancelAnimationFrame(secondFrame)
    }
  }, [query])

  const navigate = (direction: 1 | -1) => {
    const matches = matchedFields()
    setMatchCount(matches.length)
    if (matches.length === 0) return
    const nextIndex = activeIndex < 0
      ? (direction === 1 ? 0 : matches.length - 1)
      : (activeIndex + direction + matches.length) % matches.length
    setActiveIndex(nextIndex)
    focusResult(matches[nextIndex])
  }

  const clear = () => {
    clearCurrentResult()
    onQueryChange('')
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      navigate(event.shiftKey ? -1 : 1)
    } else if (event.key === 'Escape') {
      event.preventDefault()
      clear()
    }
  }

  return (
    <Surface className="p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.parameterSearch}</p>
          <p className="mt-1 text-sm text-slate-500">{labels.parameterSearchDesc}</p>
        </div>
        <p className="text-xs text-slate-500">{labels.parameterSearchKeys}</p>
      </div>
      <div className="mt-4 flex items-center gap-2">
        <TextInput
          type="text"
          value={query}
          onChange={event => onQueryChange(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={labels.parameterSearchPlaceholder}
          leadingIcon={<Search className="h-4 w-4" />}
          aria-label={labels.parameterSearch}
          className="min-w-0 flex-1"
        />
        {query && (
          <>
            <span className={`min-w-[76px] text-center text-xs ${matchCount > 0 ? 'text-slate-500' : 'text-amber-500'}`}>
              {matchCount > 0
                ? `${activeIndex >= 0 ? activeIndex + 1 : 0} / ${matchCount}`
                : labels.parameterSearchNoResults}
            </span>
            <Button type="button" onClick={() => navigate(-1)} disabled={matchCount === 0} variant="secondary" size="icon" title={labels.parameterSearchPrevious} aria-label={labels.parameterSearchPrevious}>
              <ChevronUp className="h-4 w-4" />
            </Button>
            <Button type="button" onClick={() => navigate(1)} disabled={matchCount === 0} variant="secondary" size="icon" title={labels.parameterSearchNext} aria-label={labels.parameterSearchNext}>
              <ChevronDown className="h-4 w-4" />
            </Button>
            <Button type="button" onClick={clear} variant="subtle" size="icon" title={labels.parameterSearchClear} aria-label={labels.parameterSearchClear}>
              <X className="h-4 w-4" />
            </Button>
          </>
        )}
      </div>
    </Surface>
  )
}
