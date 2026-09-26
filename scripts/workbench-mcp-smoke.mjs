// Native acceptance through the installed stdio executable, never through UI clicks.
// Isolated profile + installation path; no keys, model calls or business files.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, copyFile, readdir } from "node:fs/promises";
import { createServer } from "node:net";
import { createInterface } from "node:readline";
import { resolve, join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
const repo = fileURLToPath(new URL("..", import.meta.url));
const parent = process.env.WORKBENCH_TEST_ROOT;
if (!parent)
  throw Error("Set WORKBENCH_TEST_ROOT to a dedicated scratch directory");
await mkdir(parent, { recursive: true });
const root = await mkdtemp(join(parent, "mcp 本地 "));
const profile = join(root, "profile");
const installation = join(root, "Application Space");
await mkdir(installation);
const suffix = process.platform === "win32" ? ".exe" : "";
const hostSource = resolve(
  process.env.WORKBENCH_TEST_EXE ||
    join(repo, "target/debug/nuphus-workbench" + suffix),
);
const bridgeSource = resolve(
  process.env.WORKBENCH_TEST_MCP ||
    join(dirname(hostSource), "nuphus-workbench-mcp" + suffix),
);
// Packaged macOS apps must stay in their bundle for frameworks/resources lookup.
let bridge = bridgeSource;
if (process.platform === "win32") {
  for (const source of [
    hostSource,
    bridgeSource,
    ...(await readdir(dirname(hostSource)))
      .filter((p) => p.endsWith(".dll"))
      .map((p) => join(dirname(hostSource), p)),
  ]) {
    const target = join(
      installation,
      source.slice(
        source.lastIndexOf(process.platform === "win32" ? "\\" : "/") + 1,
      ),
    );
    await copyFile(source, target);
  }
  bridge = join(installation, "nuphus-workbench-mcp.exe");
}
const occupied = createServer((socket) => socket.end());
await new Promise((resolve, reject) => {
  occupied.once("error", reject);
  occupied.listen(0, "127.0.0.1", resolve);
});
const port = occupied.address().port;
let hostPid;
const children = [];
const pause = (ms) => new Promise((r) => setTimeout(r, ms));
function client() {
  const env = {
    ...process.env,
    NUPHUS_WORKBENCH_DATA_DIR: profile,
    NUPHUS_WORKBENCH_PORT: String(port),
  };
  delete env.NUPHUS_WORKBENCH_URL;
  delete env.NUPHUS_WORKBENCH_TOKEN;
  const child = spawn(bridge, ["serve"], {
    env,
    windowsHide: true,
    stdio: ["pipe", "pipe", "pipe"],
  });
  children.push(child);
  let id = 0;
  const pending = new Map();
  let failure;
  const lines = createInterface({ input: child.stdout });
  const fail = (error) => {
    failure = error;
    for (const item of pending.values()) {
      clearTimeout(item.timer);
      item.reject(error);
    }
    pending.clear();
  };
  lines.on("line", (line) => {
    let value;
    try {
      value = JSON.parse(line);
    } catch {
      fail(Error("MCP stdout contains non-JSON data"));
      return;
    }
    const entry = pending.get(value.id);
    if (!entry) return;
    pending.delete(value.id);
    clearTimeout(entry.timer);
    value.error
      ? entry.reject(Error(JSON.stringify(value.error)))
      : entry.resolve(value.result);
  });
  child.on("error", fail);
  child.on("exit", (code) => fail(Error(`MCP exited ${code}`)));
  child.stderr.on("data", () => {});
  const rpc = (method, params) =>
    new Promise((resolve, reject) => {
      if (failure) {
        reject(failure);
        return;
      }
      const callId = ++id;
      const timer = setTimeout(() => {
        pending.delete(callId);
        reject(Error(`Timeout: ${method}`));
      }, 40000);
      pending.set(callId, { resolve, reject, timer });
      child.stdin.write(
        JSON.stringify({
          jsonrpc: "2.0",
          id: callId,
          method,
          ...(params ? { params } : {}),
        }) + "\n",
      );
    });
  return {
    rpc,
    async init() {
      await rpc("initialize", {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "workbench-acceptance", version: "1" },
      });
      child.stdin.write(
        JSON.stringify({
          jsonrpc: "2.0",
          method: "notifications/initialized",
        }) + "\n",
      );
      const tools = await rpc("tools/list");
      assert.ok(tools.tools.some((t) => t.name === "workflow_run"));
    },
    async call(name, args = {}) {
      const result = await rpc("tools/call", {
        name: name.replaceAll(".", "_"),
        arguments: args,
      });
      if (result.isError) throw Error(JSON.stringify(result.structuredContent));
      return result.structuredContent.result;
    },
  };
}
async function stopHost() {
  if (!hostPid) return;
  try {
    process.kill(hostPid);
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
  hostPid = undefined;
  await pause(1000);
}
try {
  console.log(
    JSON.stringify({ stage: "cold_start_with_HTTP_port_occupied", root, port }),
  );
  const a = client(),
    b = client();
  await Promise.all([a.init(), b.init()]);
  const first = await a.call("system.capabilities");
  hostPid = first.host.process_id;
  assert.ok(hostPid);
  assert.equal(
    (await b.call("system.capabilities")).host.process_id,
    hostPid,
    "Concurrent clients must share one host",
  );
  if (process.env.WORKBENCH_TEST_EXPECT_AUTH_REQUIRED === "1") {
    assert.equal(first.auth_required, true);
    await assert.rejects(a.call("project.list"));
    const previousPid = hostPid;
    await stopHost();
    hostPid = (await a.call("system.capabilities")).host.process_id;
    assert.notEqual(hostPid, previousPid);
    await assert.rejects(b.call("project.list"));
    console.log(
      JSON.stringify({
        stage: "PASS",
        checks: [
          "automatic tray startup",
          "concurrent host reuse",
          "HTTP port conflict independence",
          "unauthenticated business blocked",
          "same-client reconnect remains locked",
        ],
        root,
      }),
    );
  } else {
    const project_id = (await a.call("project.list"))[0].project_id;
    const draft = await a.call("workflow.import", {
      project_id,
      name: "MCP smoke",
      document: {
        id: "import-id",
        name: "MCP smoke",
        status: "Draft",
        inputs: [],
        steps: [{ id: "wait", name: "Wait", do: { sleep: 0.05 } }],
      },
    });
    const scope = { project_id, workflow_id: draft.workflow_id };
    assert.equal(draft.authoring_mode, "external");
    assert.equal(
      (await b.call("canvas.get", scope)).workflow_id,
      draft.workflow_id,
    );
    const version = await a.call("workflow.save", {
      ...scope,
      revision: draft.revision,
    });
    const request = {
      project_id,
      version_id: version.version_id,
      request_id: "mcp-smoke-once",
    };
    const run = await a.call("workflow.run", request);
    for (let i = 0; i < 40; i++) {
      const state = await a.call("run.get", { project_id, run_id: run.run_id });
      if (state.status === "completed") break;
      assert.ok(
        !["failed", "interrupted"].includes(state.status),
        JSON.stringify(state.result),
      );
      await pause(250);
    }
    assert.equal(
      (await a.call("run.get", { project_id, run_id: run.run_id })).status,
      "completed",
    );
    const trace = await b.call("run.steps", { project_id, run_id: run.run_id });
    assert.ok(
      trace.invocations.some(
        (s) => s.step_id === "wait" && s.status === "success",
      ),
    );
    const previousPid = hostPid;
    await stopHost();
    console.log("Restarting through the SAME MCP process/configuration");
    hostPid = (await a.call("system.capabilities")).host.process_id;
    assert.notEqual(hostPid, previousPid);
    assert.equal((await b.call("workflow.run", request)).run_id, run.run_id);
    assert.equal((await a.call("run.list", { project_id })).length, 1);
    console.log(
      JSON.stringify({
        stage: "PASS",
        checks: [
          "stdio initialize/discovery",
          "automatic tray startup",
          "concurrent client reuse",
          "HTTP port conflict independence",
          "Unicode/spaced installation path",
          "shared canvas and workflow execution",
          "step evidence",
          "same-client reconnection",
          "no replay",
        ],
        root,
        run_id: run.run_id,
      }),
    );
  }
} finally {
  for (const child of children) {
    child.stdin.end();
    child.kill();
  }
  await stopHost();
  occupied.close();
}
