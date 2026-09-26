// Real native-host acceptance. Uses a NEW isolated project/profile, no model calls.
// Build Workbench first. Windows default; override WORKBENCH_TEST_EXE for other OSes.
// A temporary Notepad document may be opened separately for the read-only RPA query.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir } from "node:fs/promises";
import { resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
const repo = fileURLToPath(new URL("..", import.meta.url));
const parent = process.env.WORKBENCH_TEST_ROOT;
if (!parent)
  throw Error(
    "Set WORKBENCH_TEST_ROOT to an explicit scratch directory (E: on this machine)",
  );
await mkdir(parent, { recursive: true });
const root = await mkdtemp(join(parent, "schedule-"));
const exe =
  process.env.WORKBENCH_TEST_EXE ||
  resolve(repo, "target/debug/nuphus-workbench.exe");
const port = process.env.WORKBENCH_TEST_PORT || "47739";
const base = `http://127.0.0.1:${port}`;
let child;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function call(operation, args = {}) {
  const response = await fetch(`${base}/api/v1/${operation}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(10000),
  });
  const body = await response.json();
  if (body.error) throw Error(`${body.error.code}: ${body.error.message}`);
  return body.result;
}
async function start() {
  child = spawn(exe, ["--background"], {
    cwd: resolve(repo, "target/debug"),
    windowsHide: true,
    stdio: "ignore",
    env: {
      ...process.env,
      NUPHUS_WORKBENCH_DATA_DIR: root,
      NUPHUS_WORKBENCH_PORT: port,
    },
  });
  child.on("error", (e) => console.error(e.message));
  for (let i = 0; i < 40; i++) {
    try {
      await call("project.list");
      return;
    } catch {
      if (child.exitCode !== null) throw Error(`Host exited ${child.exitCode}`);
      await sleep(500);
    }
  }
  throw Error("Isolated host did not start");
}
async function stop() {
  if (child && child.exitCode === null) {
    const exited = new Promise((r) => child.once("exit", r));
    child.kill();
    await exited;
  }
  await sleep(500);
}
try {
  await start();
  assert.equal((await call("system.capabilities")).host.scheduled_rpa, true);
  const project_id = (await call("project.list"))[0].project_id;
  let draft = await call("workflow.import", {
    project_id,
    name: "Schedule smoke",
    document: {
      id: "replaced-by-import",
      name: "Schedule smoke",
      status: "Draft",
      steps: [
        { id: "wait", name: "Temporary wait", do: { sleep: 0.05 } },
        {
          id: "observe",
          name: "Read only temporary app query",
          do: {
            tool: "desktop_targets_list",
            with: {
              query:
                process.env.WORKBENCH_TEST_WINDOW_QUERY ||
                "schedule-smoke-notepad",
            },
          },
        },
      ],
      inputs: [],
      schedule: null,
    },
  });
  const scope = { project_id, workflow_id: draft.workflow_id };
  const config = {
    cron: "* * * * *",
    timezone: "UTC",
    enabled: true,
    interval_minutes: 1,
  };
  draft = (
    await call("workflow.schedule.set", {
      ...scope,
      revision: draft.revision,
      config,
      inputs: {},
    })
  ).draft;
  const binding = (await call("workflow.schedule.list", { project_id }))[0];
  assert.ok(binding.next_at > Date.now());
  draft = (
    await call("canvas.update", {
      ...scope,
      revision: draft.revision,
      operations: [
        {
          op: "replace_document",
          document: {
            ...draft.document,
            name: "Latest saved scheduled workflow",
          },
        },
      ],
    })
  ).draft;
  console.log(
    JSON.stringify({
      stage: "waiting_for_real_occurrence",
      project_id,
      workflow_id: draft.workflow_id,
      next_at: binding.next_at,
      root,
    }),
  );
  let rows = [];
  for (let i = 0; i < 95; i++) {
    rows = await call("workflow.schedule.history", scope);
    if (rows[0]?.status === "completed") break;
    if (
      rows[0] &&
      ["failed", "skipped", "interrupted"].includes(rows[0].status)
    )
      throw Error(
        JSON.stringify({
          occurrence: rows[0],
          run: rows[0].run_id
            ? await call("run.get", { project_id, run_id: rows[0].run_id })
            : null,
        }),
      );
    if (i % 15 === 0) console.log(`Waiting for native timer (${i}s)`);
    await sleep(1000);
  }
  assert.equal(rows.length, 1);
  assert.equal(rows[0].status, "completed");
  const run = await call("run.get", { project_id, run_id: rows[0].run_id });
  assert.equal(run.source, "schedule");
  assert.equal(run.snapshots[0].name, "Latest saved scheduled workflow");
  const trace = await call("run.steps", { project_id, run_id: run.run_id });
  assert.ok(
    trace.invocations.some(
      (step) => step.step_id === "wait" && step.status === "success",
    ),
  );
  assert.ok(
    trace.invocations.some(
      (step) => step.step_id === "observe" && step.status === "success",
    ),
  );
  const observation = trace.invocations.find(
    (step) => step.step_id === "observe",
  );
  const evidence = await call("run.steps", {
    project_id,
    run_id: run.run_id,
    invocation_id: observation.id,
  });
  assert.ok(
    evidence.output,
    "Native observation must retain its actual output",
  );
  if (process.env.WORKBENCH_TEST_REQUIRE_WINDOW === "1") {
    assert.ok(
      evidence.output.includes(
        process.env.WORKBENCH_TEST_WINDOW_QUERY || "schedule-smoke-notepad",
      ),
      "The observation must find the temporary application, not merely return success",
    );
  }
  const next = (await call("workflow.schedule.list", { project_id }))[0]
    .next_at;
  await stop();
  await start();
  await sleep(2500);
  assert.equal(
    (await call("workflow.schedule.list", { project_id }))[0].next_at,
    next,
  );
  assert.equal((await call("workflow.schedule.history", scope)).length, 1);
  assert.equal((await call("run.list", { project_id })).length, 1);
  draft = await call("canvas.get", scope);
  draft = (
    await call("workflow.schedule.set", {
      ...scope,
      revision: draft.revision,
      config: { ...config, enabled: false },
      inputs: {},
    })
  ).draft;
  assert.equal(
    (await call("workflow.schedule.list", { project_id }))[0].next_at,
    null,
  );
  await call("workflow.schedule.remove", {
    ...scope,
    revision: draft.revision,
  });
  assert.equal(
    (await call("workflow.schedule.list", { project_id })).length,
    0,
  );
  assert.equal((await call("workflow.schedule.history", scope)).length, 1);
  console.log(
    JSON.stringify({
      stage: "PASS",
      checks: [
        "actual one-minute trigger",
        "read-only desktop RPA",
        "latest saved snapshot",
        "step evidence",
        "restart preserves next occurrence",
        "no duplicate run",
        "disable/remove",
      ],
      run_id: run.run_id,
      root,
    }),
  );
} finally {
  await stop();
}
