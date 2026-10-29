import { useEffect, useSyncExternalStore } from 'react'
import type { ToolSchema } from '../../core/types'
import { wfTools } from '../lib/api'

// One observable cache keeps the palette, inspector and drop handler in sync after retry.
// undefined = loading, null = failed, [] = a successful empty registry.
let tools: ToolSchema[] | null | undefined
let inflight: Promise<ToolSchema[] | null> | null = null
const listeners = new Set<() => void>()
const notify = () => listeners.forEach(listener => listener())
const subscribe = (listener: () => void) => {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function loadToolsOnce(): Promise<ToolSchema[] | null> {
  if (tools != null) return Promise.resolve(tools)
  if (!inflight) {
    tools = undefined
    inflight = wfTools()
      .then(result => {
        if (!Array.isArray(result)) throw new Error('Tool registry unavailable')
        tools = result
        return result
      })
      .catch(() => {
        tools = null
        return null
      })
      .finally(() => {
        inflight = null
        notify()
      })
    notify()
  }
  return inflight
}

export function useToolRegistry() {
  const snapshot = useSyncExternalStore(subscribe, () => tools)
  useEffect(() => {
    void loadToolsOnce()
  }, [])
  return snapshot
}
