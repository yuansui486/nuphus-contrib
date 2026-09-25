import { beforeEach, describe, expect, it, vi } from 'vitest'
import { canvasBackend, type Draft } from './api'
const ipc = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: ipc }))
const fixture = (): Draft => ({
  project_id: 'project',
  workflow_id: 'flow',
  revision: 7,
  layout_revision: 2,
  authoring_mode: 'external',
  document: { id: 'flow', name: 'Test', steps: [], status: 'Draft' } as Draft['document'],
  layout: {},
  updated_at: 0,
})
beforeEach(() => {
  ipc.mockReset()
})

describe('Workbench canvas backend', () => {
  it('keeps content revision unchanged on layout updates and never uses legacy wf commands', async () => {
    const draft = fixture()
    const backend = canvasBackend(draft, vi.fn(), vi.fn())
    ipc.mockImplementation(async (_command, { operation, args }) => {
      if (operation === 'canvas.layout_update')
        return { ...draft, revision: 10, layout_revision: 3, layout: args.layout }
      if (operation === 'canvas.update')
        return { draft: { ...draft, revision: 8 }, diagnostics: { passed: true } }
    })
    await backend.wfLayoutSave('flow', { test: true })
    await backend.wfSave(draft.document)
    expect(ipc).toHaveBeenLastCalledWith(
      'workbench_call',
      expect.objectContaining({
        operation: 'canvas.update',
        args: expect.objectContaining({ revision: 7 }),
      }),
    )
    expect(ipc.mock.calls.every(([command]) => command === 'workbench_call')).toBe(true)
  })
  it('preserves the original base on conflict instead of silently retrying with a newer revision', async () => {
    const draft = fixture(),
      changed = vi.fn()
    const backend = canvasBackend(draft, changed, vi.fn())
    ipc.mockRejectedValue({
      code: 'revision_conflict',
      message: 'Reload first',
      details: { current_revision: 9 },
    })
    await expect(backend.wfSave(draft.document)).rejects.toThrow('revision_conflict')
    expect(changed).not.toHaveBeenCalled()
    await expect(backend.wfSave(draft.document)).rejects.toThrow('revision_conflict')
    expect(ipc.mock.calls[1][1].args.revision).toBe(7)
  })
  it('reuses request identity after a lost start reply, but intentionally new starts get a new ID', async () => {
    const backend = canvasBackend(fixture(), vi.fn(), vi.fn())
    let starts = 0
    ipc.mockImplementation(async (_command, { operation }) => {
      if (operation === 'workflow.save') return { version_id: 'published' }
      if (operation === 'workflow.run' && ++starts === 1) throw new Error('connection closed')
      return { run_id: 'run' }
    })
    await expect(backend.wfRun('flow')).rejects.toThrow('connection closed')
    await backend.wfRun('flow')
    await backend.wfRun('flow')
    const requests = ipc.mock.calls
      .filter(([, value]) => value.operation === 'workflow.run')
      .map(([, value]) => value.args)
    expect(requests[0].request_id).toBe(requests[1].request_id)
    expect(requests[2].request_id).not.toBe(requests[1].request_id)
  })
  it('serializes rapid layout saves with independent layout revisions', async () => {
    const draft = fixture(),
      backend = canvasBackend(draft, vi.fn(), vi.fn())
    ipc.mockImplementation(async (_command, { args }) => ({
      ...draft,
      layout: args.layout,
      layout_revision: args.layout_revision + 1,
    }))
    await Promise.all([
      backend.wfLayoutSave('flow', { a: 1 }),
      backend.wfLayoutSave('flow', { a: 2 }),
    ])
    expect(ipc.mock.calls.map(([, value]) => value.args.layout_revision)).toEqual([2, 3])
  })
  it('does not repeat a debug action after a lost acknowledgement', async () => {
    const backend = canvasBackend(fixture(), vi.fn(), vi.fn())
    const request = {
      workflow_id: 'flow',
      selected_step_id: 'wait',
      mode: 'node' as const,
      steps: [],
      inputs: [],
      source: { kind: 'manual' },
      variables: {},
      runtime_inputs: {},
      use_retry_policy: false,
    }
    ipc.mockRejectedValueOnce(new Error('lost reply')).mockResolvedValue({ run_id: 'debug-run' })
    await expect(backend.wfDebugRun(request)).rejects.toThrow('lost reply')
    await expect(backend.wfDebugRun({ ...request, selected_step_id: 'another' })).rejects.toThrow(
      'start_uncertain',
    )
    await backend.wfDebugRun(request)
    expect(ipc.mock.calls[0][1].args.request_id).toBe(ipc.mock.calls[1][1].args.request_id)
  })
})
