import { invoke } from '@tauri-apps/api/core'
import type { CanvasBackend } from '../main-window/workflow-canvas/CanvasBackend'
import type { WorkflowIR } from '../main-window/workflow-canvas/types'
import type { ValidationReport, WorkflowRunTrace } from '../main-window/lib/api'

export interface Project {
  project_id: string
  name: string
  directory: string
}
export interface Draft {
  project_id: string
  workflow_id: string
  revision: number
  layout_revision: number
  authoring_mode: 'internal' | 'external'
  document: WorkflowIR
  layout: Record<string, unknown>
  updated_at: number
}
export interface Run {
  run_id: string
  workflow_id: string
  version_id: string
  status: string
  inputs: Record<string, unknown>
  result: unknown
  created_at: number
  pending_request?: { request_id: string; step_id: string; prompt: string } | null
  debug?: Record<string, unknown> | null
}
export interface Client {
  client_id: string
  name: string
  projects: string[]
  capabilities: string[]
  revoked: boolean
}
export interface Endpoint {
  status: string
  url?: string
  mcp_url?: string
  message?: string
}
export class WorkbenchError extends Error {
  constructor(
    public code: string,
    message: string,
    public details?: unknown,
  ) {
    super(`${code}: ${message}`)
  }
}
export async function call<T>(operation: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    return await invoke<T>('workbench_call', { operation, args })
  } catch (error) {
    if (error && typeof error === 'object' && 'message' in error) {
      if ('code' in error)
        throw new WorkbenchError(
          String(error.code),
          String(error.message),
          'details' in error ? error.details : undefined,
        )
      throw error
    }
    throw error
  }
}
export const clients = <T>(action: string, args: Record<string, unknown> = {}) =>
  invoke<T>('workbench_clients', { action, args })

/** A mounted editor owns its base revision. Background polling must never move
 * this base forward while there are unsaved local edits. Only successful writes
 * here advance it; accepting an external revision creates a fresh editor. */
export function canvasBackend(
  initial: Draft,
  changed: (draft: Draft, kind?: 'layout') => void,
  started: (id: string) => void,
): CanvasBackend {
  let current = initial
  let layoutQueue: Promise<unknown> = Promise.resolve()
  let pendingRun: {
    project_id: string
    version_id: string
    request_id: string
    inputs: Record<string, unknown>
  } | null = null
  const scope = { project_id: initial.project_id, workflow_id: initial.workflow_id }
  return {
    versioned: true,
    scheduling: false,
    debugging: true,
    generation: initial.authoring_mode === 'internal',
    wfGetRaw: async () => current.document as unknown as Record<string, unknown>,
    wfLayoutGet: async () => current.layout,
    wfLayoutSave: async (_id, layout) => {
      const operation = layoutQueue
        .catch(() => {})
        .then(async () => {
          const draft = await call<Draft>('canvas.layout_update', {
            ...scope,
            layout_revision: current.layout_revision,
            layout,
          })
          // A concurrent content save owns its content revision; only merge layout.
          current = { ...current, layout: draft.layout, layout_revision: draft.layout_revision }
          changed(current, 'layout')
        })
      layoutQueue = operation
      await operation
    },
    wfSave: async document => {
      const response = await call<{ draft: Draft; diagnostics: ValidationReport }>(
        'canvas.update',
        {
          ...scope,
          revision: current.revision,
          operations: [{ op: 'replace_document', document }],
        },
      )
      current = {
        ...response.draft,
        layout: current.layout,
        layout_revision: current.layout_revision,
      }
      changed(current)
      return { saved: true, report: response.diagnostics }
    },
    wfValidate: document => call('workflow.validate', { ...scope, document }),
    wfRun: async (_id, _fresh, inputs = {}) => {
      if (pendingRun && JSON.stringify(pendingRun.inputs) !== JSON.stringify(inputs)) {
        throw new WorkbenchError(
          'start_uncertain',
          'A previous start has not been acknowledged. Inspect Runs or retry with the same inputs before starting a different run.',
        )
      }
      if (!pendingRun) {
        const version = await call<{ version_id: string }>('workflow.save', {
          ...scope,
          revision: current.revision,
        })
        pendingRun = {
          project_id: scope.project_id,
          version_id: version.version_id,
          request_id: crypto.randomUUID(),
          inputs,
        }
      }
      let run: { run_id: string }
      try {
        run = await call<{ run_id: string }>('workflow.run', pendingRun)
      } catch (error) {
        // A structured rejection is definitive. A lost transport reply is not.
        if (error instanceof WorkbenchError) pendingRun = null
        throw error
      }
      pendingRun = null
      started(run.run_id)
      return run.run_id
    },
    listWorkflows: async () =>
      (await call<Draft[]>('workflow.list', { project_id: scope.project_id })).map(draft => ({
        id: draft.workflow_id,
        title: draft.document.name,
        steps: draft.document.steps,
        tags: [],
        created_at: draft.updated_at / 1000,
        updated_at: draft.updated_at / 1000,
        run_count: 0,
        status: 'draft' as const,
        inputs: draft.document.inputs,
      })),
    wfTraceList: async (_id, debug) => {
      const runs = await call<Run[]>('run.list', { project_id: scope.project_id })
      const traces = await Promise.all(
        runs
          .filter(run => run.workflow_id === scope.workflow_id && Boolean(run.debug) === debug)
          .map(run =>
            call<WorkflowRunTrace | null>('run.steps', {
              project_id: scope.project_id,
              run_id: run.run_id,
            }),
          ),
      )
      return traces.filter((trace): trace is WorkflowRunTrace => trace !== null)
    },
    wfTraceRead: (_id, runId, _debug, invocationId) =>
      call('run.steps', {
        project_id: scope.project_id,
        run_id: runId,
        invocation_id: invocationId,
      }),
    wfDebugRun: async request => {
      const { inputs, ...parameters } = request
      const run = await call<{ run_id: string }>('workflow.debug', {
        ...parameters,
        ...scope,
        revision: current.revision,
        input_specs: inputs,
        request_id: crypto.randomUUID(),
      })
      started(run.run_id)
      return run
    },
    wfDebugControl: async (_id, runId, action) => {
      await call(`run.${action}`, { project_id: scope.project_id, run_id: runId })
    },
  }
}
