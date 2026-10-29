// Node 22+. Start Workbench, then run this example; no client setup required.
// Reuses one wait-only demo per project; no desktop/filesystem effects.
// NUPHUS_WORKBENCH_WORKFLOW selects an existing wait-only workflow instead.
// Set NUPHUS_WORKBENCH_CREATE_REQUEST_ID to a new UUID for a separate demo.
import { randomUUID } from "node:crypto";
const base = process.env.NUPHUS_WORKBENCH_URL || "http://127.0.0.1:47731";
const token = process.env.NUPHUS_WORKBENCH_TOKEN;
async function call(operation, args = {}) {
  const response = await fetch(`${base}/api/v1/${operation}`, {
    method: "POST",
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      "Content-Type": "application/json",
    },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  });
  const body = await response.json();
  if (!response.ok || body.error) throw new Error(JSON.stringify(body.error));
  return body.result;
}
const projects = await call("project.list");
const project_id =
  process.env.NUPHUS_WORKBENCH_PROJECT || projects[0]?.project_id;
if (!project_id)
  throw new Error("No permitted project; register one in Workbench");
const workflow_id = process.env.NUPHUS_WORKBENCH_WORKFLOW;
const steps = [{ id: "wait", name: "Wait briefly", do: { sleep: 0.05 } }];
// Preserve this key and the arguments across uncertain retries/restarts.
// The fixed, example-specific default intentionally reuses the project's demo.
const draft = workflow_id
  ? await call("canvas.get", { project_id, workflow_id })
  : await call("workflow.import", {
      project_id,
      name: "External client smoke test",
      request_id:
        process.env.NUPHUS_WORKBENCH_CREATE_REQUEST_ID ||
        "example:workbench-client:wait:v1",
      document: {
        name: "External client smoke test",
        status: "Draft",
        steps,
        doc: null,
        schedule: null,
        inputs: [],
        dry_run: false,
      },
    });
const scope = { project_id, workflow_id: draft.workflow_id };
console.log("Workflow (reuse this ID):", draft.workflow_id);
// Never overwrite an existing canvas or execute an edited demo's real actions.
const existingSteps = draft.document.steps;
if (
  existingSteps?.length !== 1 ||
  existingSteps[0].do?.sleep !== 0.05 ||
  Object.keys(existingSteps[0].do).length !== 1
)
  throw new Error(
    "The selected workflow is no longer the wait-only demo; use a new creation request ID for a separate demo",
  );
const version = await call("workflow.save", {
  ...scope,
  revision: draft.revision,
});
// Preserve this object if a transport failure makes the response uncertain.
const request = {
  project_id,
  version_id: version.version_id,
  request_id: randomUUID(),
  inputs: {},
};
const run = await call("workflow.run", request);
console.log("Run:", run.run_id);
let cursor = 0;
for (let attempt = 0; attempt < 60; attempt++) {
  const events = await call("run.events", {
    project_id,
    run_id: run.run_id,
    after: cursor,
  });
  for (const event of events) {
    cursor = event.cursor;
    console.log(event.kind);
  }
  const state = await call("run.get", { project_id, run_id: run.run_id });
  if (
    ["completed", "failed", "cancelled", "interrupted"].includes(state.status)
  ) {
    console.log(state.status, state.result);
    if (state.status !== "completed") process.exitCode = 1;
    break;
  }
  if (attempt === 59)
    throw new Error(
      `Timed out observing ${run.run_id}; query the existing run, do not restart it`,
    );
  await new Promise((resolve) => setTimeout(resolve, 500));
}
