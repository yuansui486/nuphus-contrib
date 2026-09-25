// Real Windows WebView2 + native-host smoke test (Node 22+; debug builds only).
// Launch the Workbench with an isolated NUPHUS_WORKBENCH_DATA_DIR and
// Windows WebView2 additionalBrowserArgs=--remote-debugging-port=9227 in a
// LOCAL test-only Tauri config (never add a debugging port to release config).
// No desktop input or business files are touched. The short wait workflow and
// revoked test client remain in the explicitly selected scratch project.
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
const cdpPort = process.env.WORKBENCH_TEST_CDP_PORT || "9227";
const pages = await (await fetch(`http://127.0.0.1:${cdpPort}/json`)).json();
const page = pages.find(
  (page) =>
    page.type === "page" &&
    /(?:tauri\.localhost|localhost:5176)\/(?:index.html)?$/.test(page.url),
);
if (!page)
  throw new Error(
    "Start a dedicated Workbench debug instance before this test",
  );
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.onopen = resolve;
  socket.onerror = reject;
});
const pending = new Map();
let sequence = 0;
socket.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (!message.id) return;
  const handlers = pending.get(message.id);
  pending.delete(message.id);
  if (message.error) handlers?.reject(new Error(message.error.message));
  else handlers?.resolve(message.result);
};
async function evaluate(expression) {
  const id = ++sequence;
  const result = await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      pending.delete(id);
      reject(new Error("CDP evaluation timed out"));
    }, 30_000);
    pending.set(id, {
      resolve: (result) => {
        clearTimeout(timeout);
        resolve(result);
      },
      reject: (error) => {
        clearTimeout(timeout);
        reject(error);
      },
    });
    socket.send(
      JSON.stringify({
        id,
        method: "Runtime.evaluate",
        params: { expression, awaitPromise: true, returnByValue: true },
      }),
    );
  });
  if (result.exceptionDetails)
    throw new Error(
      result.exceptionDetails.exception?.description || "WebView exception",
    );
  return result.result.value;
}
const invoke = (command, args) =>
  evaluate(
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`,
  );
const ui = (operation, args = {}) =>
  invoke("workbench_call", { operation, args });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let client;
let projectId;
const ownedRuns = [];
try {
  const capability = await ui("system.capabilities");
  assert.equal(capability.host.edition, "workbench");
  assert.match(await evaluate("document.body.innerText"), /工作流|Workflows/);
  const projects = await ui("project.list");
  const project_id = projects[0].project_id;
  projectId = project_id;
  client = await invoke("workbench_clients", {
    action: "create",
    args: {
      name: "Native smoke test",
      projects: [project_id],
      capabilities: ["read", "edit", "run", "respond"],
    },
  });
  const endpoint = await invoke("workbench_clients", {
    action: "status",
    args: {},
  });
  assert.equal(endpoint.status, "listening");
  const http = async (operation, args = {}) => {
    const response = await fetch(`${endpoint.url}/api/v1/${operation}`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${client.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(args),
      signal: AbortSignal.timeout(30_000),
    });
    const body = await response.json();
    if (body.error)
      throw new Error(`${body.error.code}: ${body.error.message}`);
    if (
      body.result?.run_id &&
      ["workflow.run", "workflow.debug"].includes(operation)
    )
      ownedRuns.push(body.result.run_id);
    return body.result;
  };
  const draft = await http("canvas.create", {
    project_id,
    name: "Native host smoke test",
  });
  assert.equal(draft.authoring_mode, "external");
  const scope = { project_id, workflow_id: draft.workflow_id };
  const edited = await http("canvas.update", {
    ...scope,
    revision: draft.revision,
    operations: [
      {
        op: "add_step",
        parent_id: null,
        lane: "main",
        index: 0,
        step: { id: "wait", name: "Smoke wait", do: { sleep: 0.1 } },
      },
    ],
  });
  assert.equal(edited.diagnostics.passed, true);
  await http("canvas.open", { ...scope, focus: false });
  for (let attempt = 0; attempt < 30; attempt++) {
    if (
      (await evaluate("document.body.innerText")).includes(
        "Native host smoke test",
      )
    )
      break;
    await sleep(200);
  }
  const visible = await evaluate("document.body.innerText");
  assert.match(visible, /Native host smoke test/);
  assert.equal(
    await evaluate('Boolean(document.querySelector(".wb-authoring"))'),
    false,
  );
  const version = await http("workflow.save", {
    ...scope,
    revision: edited.draft.revision,
  });
  const request = {
    project_id,
    version_id: version.version_id,
    request_id: crypto.randomUUID(),
    inputs: {},
  };
  const run = await http("workflow.run", request);
  assert.equal((await http("workflow.run", request)).run_id, run.run_id);
  let status;
  for (let attempt = 0; attempt < 60; attempt++) {
    status = await http("run.get", { project_id, run_id: run.run_id });
    if (
      ["completed", "failed", "cancelled", "interrupted"].includes(
        status.status,
      )
    )
      break;
    await sleep(100);
  }
  assert.equal(status.status, "completed", JSON.stringify(status.result));
  const trace = await http("run.steps", { project_id, run_id: run.run_id });
  assert.equal(trace.run_id, run.run_id);
  assert(
    (await http("run.events", { project_id, run_id: run.run_id })).some(
      (event) => event.kind === "run.completed",
    ),
  );
  async function until(run_id, expected) {
    for (let attempt = 0; attempt < 100; attempt++) {
      const state = await http("run.get", { project_id, run_id });
      if (state.status === expected) return state;
      if (["failed", "cancelled", "interrupted"].includes(state.status))
        throw new Error(JSON.stringify(state.result));
      await sleep(100);
    }
    throw new Error(
      `Run did not reach ${expected}; inspect existing run ${run_id}`,
    );
  }
  const debug = await http("workflow.debug", {
    ...scope,
    revision: edited.draft.revision,
    selected_step_id: "wait",
    mode: "node",
    request_id: crypto.randomUUID(),
  });
  await until(debug.run_id, "completed");
  assert.equal(
    (await http("run.steps", { project_id, run_id: debug.run_id })).run_id,
    debug.run_id,
  );
  const waiting = await http("canvas.update", {
    ...scope,
    revision: edited.draft.revision,
    operations: [
      {
        op: "add_step",
        parent_id: null,
        lane: "main",
        index: 1,
        step: {
          id: "confirm",
          name: "Human wait",
          do: { wait: "Smoke test confirmation", auto: [] },
        },
      },
    ],
  });
  const waitVersion = await http("workflow.save", {
    ...scope,
    revision: waiting.draft.revision,
  });
  const waitRun = await http("workflow.run", {
    project_id,
    version_id: waitVersion.version_id,
    request_id: crypto.randomUUID(),
  });
  const waitingState = await until(waitRun.run_id, "awaiting_human");
  assert.equal(waitingState.pending_request.step_id, "confirm");
  await http("run.respond", {
    project_id,
    run_id: waitRun.run_id,
    request_id: waitingState.pending_request.request_id,
    decision: "continue",
  });
  await until(waitRun.run_id, "completed");
  const cancelRun = await http("workflow.run", {
    project_id,
    version_id: waitVersion.version_id,
    request_id: crypto.randomUUID(),
  });
  await until(cancelRun.run_id, "awaiting_human");
  await http("run.cancel", { project_id, run_id: cancelRun.run_id });
  await until(cancelRun.run_id, "cancelled");
  const through = await http("workflow.debug", {
    ...scope,
    revision: waiting.draft.revision,
    selected_step_id: "wait",
    mode: "through",
    request_id: crypto.randomUUID(),
  });
  await until(through.run_id, "paused");
  await http("run.cancel", { project_id, run_id: through.run_id });
  await until(through.run_id, "cancelled");
  const internal = await ui("canvas.create", {
    project_id,
    name: "Internal authoring UI smoke",
  });
  assert.equal(internal.authoring_mode, "internal");
  await ui("canvas.open", { project_id, workflow_id: internal.workflow_id });
  for (let attempt = 0; attempt < 30; attempt++) {
    if (await evaluate('Boolean(document.querySelector(".wb-authoring"))'))
      break;
    await sleep(200);
  }
  assert.equal(
    await evaluate('Boolean(document.querySelector(".wb-authoring textarea"))'),
    true,
  );
  if (process.env.WORKBENCH_TEST_SCREENSHOT) {
    const id = ++sequence;
    const shot = await new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error("Screenshot timed out")),
        10_000,
      );
      pending.set(id, {
        resolve: (result) => {
          clearTimeout(timeout);
          resolve(result);
        },
        reject,
      });
      socket.send(
        JSON.stringify({
          id,
          method: "Page.captureScreenshot",
          params: { format: "png" },
        }),
      );
    });
    await writeFile(
      process.env.WORKBENCH_TEST_SCREENSHOT,
      Buffer.from(shot.data, "base64"),
    );
  }
  console.log(
    JSON.stringify({
      passed: true,
      checks: [
        "native host",
        "workflow-first UI",
        "external canvas refresh",
        "authoring modes",
        "HTTP publish/run",
        "idempotent retry",
        "native trace",
        "durable events",
        "single-node debug",
        "through-node pause",
        "human wait response",
        "cooperative cancellation",
      ],
      run_id: run.run_id,
    }),
  );
} finally {
  for (const run_id of new Set(ownedRuns)) {
    const run = await ui("run.get", { project_id: projectId, run_id });
    if (
      !["completed", "failed", "cancelled", "interrupted"].includes(run.status)
    )
      await ui("run.cancel", { project_id: projectId, run_id });
  }
  if (client)
    await invoke("workbench_clients", {
      action: "revoke",
      args: { client_id: client.client.client_id },
    });
  socket.close();
}
