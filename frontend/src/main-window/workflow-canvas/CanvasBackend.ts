import { createContext, useContext } from 'react'
import * as api from '../lib/api'
import { wfDebugRun, wfDebugControl } from './debugSession'

/** A small edition seam: upstream editor logic stays shared, persistence does not. */
export const legacyCanvasBackend = {
  wfGetRaw: (...args: Parameters<typeof api.wfGetRaw>) => api.wfGetRaw(...args),
  wfSave: (...args: Parameters<typeof api.wfSave>) => api.wfSave(...args),
  wfValidate: (...args: Parameters<typeof api.wfValidate>) => api.wfValidate(...args),
  wfRun: (...args: Parameters<typeof api.wfRun>) => api.wfRun(...args),
  wfLayoutGet: (...args: Parameters<typeof api.wfLayoutGet>) => api.wfLayoutGet(...args),
  wfLayoutSave: (...args: Parameters<typeof api.wfLayoutSave>) => api.wfLayoutSave(...args),
  listWorkflows: (...args: Parameters<typeof api.listWorkflows>) => api.listWorkflows(...args),
  wfTraceList: (...args: Parameters<typeof api.wfTraceList>) => api.wfTraceList(...args),
  wfTraceRead: (...args: Parameters<typeof api.wfTraceRead>) => api.wfTraceRead(...args),
  wfDebugRun: (...args: Parameters<typeof wfDebugRun>) => wfDebugRun(...args),
  wfDebugControl: (...args: Parameters<typeof wfDebugControl>) => wfDebugControl(...args),
  versioned: false,
  scheduling: true,
  debugging: true,
  generation: true,
}
export type CanvasBackend = typeof legacyCanvasBackend
export const CanvasBackendContext = createContext<CanvasBackend>(legacyCanvasBackend)
export const useCanvasBackend = () => useContext(CanvasBackendContext)
