// Node 22+. Set NUPHUS_WORKBENCH_TOKEN from the Workbench UI.
// Creates a workflow containing only a short wait; no desktop/filesystem effects.
import { randomUUID } from 'node:crypto'
const base = process.env.NUPHUS_WORKBENCH_URL || 'http://127.0.0.1:47731'
const token = process.env.NUPHUS_WORKBENCH_TOKEN
if (!token) throw new Error('Create a client token in Workbench and set NUPHUS_WORKBENCH_TOKEN')
async function call(operation, args = {}) {
  const response = await fetch(`${base}/api/v1/${operation}`, {
    method: 'POST', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify(args), signal: AbortSignal.timeout(30_000),
  })
  const body = await response.json()
  if (!response.ok || body.error) throw new Error(JSON.stringify(body.error))
  return body.result
}
const projects = await call('project.list')
const project_id = process.env.NUPHUS_WORKBENCH_PROJECT || projects[0]?.project_id
if (!project_id) throw new Error('No permitted project; register one in Workbench')
const draft = await call('canvas.create', { project_id, name: 'External client smoke test' })
const scope = { project_id, workflow_id: draft.workflow_id }
const edited = await call('canvas.update', { ...scope, revision: draft.revision, operations: [
  { op: 'add_step', lane: 'main', parent_id: null, index: 0, step: { id: 'wait', name: 'Wait briefly', do: { sleep: 0.05 } } },
] })
const version = await call('workflow.save', { ...scope, revision: edited.draft.revision })
// Preserve this object if a transport failure makes the response uncertain.
const request = { project_id, version_id: version.version_id, request_id: randomUUID(), inputs: {} }
const run = await call('workflow.run', request)
console.log('Run:', run.run_id)
let cursor = 0
for (let attempt = 0; attempt < 60; attempt++) {
  const events = await call('run.events', { project_id, run_id: run.run_id, after: cursor })
  for (const event of events) { cursor = event.cursor; console.log(event.kind) }
  const state = await call('run.get', { project_id, run_id: run.run_id })
  if (['completed', 'failed', 'cancelled', 'interrupted'].includes(state.status)) {
    console.log(state.status, state.result)
    if (state.status !== 'completed') process.exitCode = 1
    break
  }
  if (attempt === 59) throw new Error(`Timed out observing ${run.run_id}; query the existing run, do not restart it`)
  await new Promise(resolve => setTimeout(resolve, 500))
}
