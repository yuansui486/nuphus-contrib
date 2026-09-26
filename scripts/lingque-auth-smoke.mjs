// Windows native acceptance. Dedicated tenant ONLY: creating a product session
// may replace an older device at the tenant's capacity. Never changes admin policy.
// Credentials arrive through process environment, not files or command arguments.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  mkdtemp,
  mkdir,
  copyFile,
  readdir,
  readFile,
  writeFile,
} from "node:fs/promises";
import { createServer } from "node:net";
import { createInterface } from "node:readline";
import { resolve, dirname, basename, join } from "node:path";
const parent = process.env.WORKBENCH_TEST_ROOT;
const tenant = process.env.LINGQUE_TEST_TENANT;
const username = process.env.LINGQUE_TEST_USERNAME;
const password = process.env.LINGQUE_TEST_PASSWORD;
if (
  process.platform !== "win32" ||
  !parent ||
  !tenant ||
  !username ||
  !password
)
  throw Error(
    "Windows: set WORKBENCH_TEST_ROOT and LINGQUE_TEST_TENANT/USERNAME/PASSWORD for a dedicated test tenant",
  );
const source = resolve(
  process.env.WORKBENCH_TEST_EXE || "target/debug/nuphus-workbench.exe",
);
await mkdir(parent, { recursive: true });
const root = await mkdtemp(join(parent, "lingque-auth-"));
const install = join(root, "Application Space");
const profile = join(root, "profile");
await mkdir(install);
await mkdir(profile);
for (const name of [
  basename(source),
  "nuphus-workbench-mcp.exe",
  ...(await readdir(dirname(source))).filter((n) => n.endsWith(".dll")),
])
  await copyFile(join(dirname(source), name), join(install, name));
async function freePort() {
  const server = createServer();
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const port = server.address().port;
  await new Promise((r) => server.close(r));
  return port;
}
const port = await freePort(),
  cdp = Number(process.env.WORKBENCH_TEST_CDP_PORT) || (await freePort());
const env = {
  ...process.env,
  NUPHUS_WORKBENCH_DATA_DIR: profile,
  NUPHUS_WORKBENCH_PORT: String(port),
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdp}`,
  WEBVIEW2_USER_DATA_FOLDER: join(profile, "webview-test"),
};
for (const key of Object.keys(env))
  if (key.startsWith("LINGQUE_TEST_")) delete env[key];
delete env.NUPHUS_WORKBENCH_URL;
delete env.NUPHUS_WORKBENCH_TOKEN;
const pause = (ms) => new Promise((r) => setTimeout(r, ms));
let app;
let socket;
const bridges = [];
let loggedIn = false;
async function start() {
  app = spawn(join(install, basename(source)), [], {
    env,
    windowsHide: true,
    stdio: "ignore",
    cwd: install,
  });
  let page;
  for (let i = 0; i < 120; i++) {
    try {
      page = (await (await fetch(`http://127.0.0.1:${cdp}/json`)).json()).find(
        (p) =>
          p.type === "page" &&
          /(?:tauri\.localhost|localhost:5176)\/(?:index.html)?$/.test(p.url),
      );
    } catch {
      /* startup */
    }
    if (page) break;
    if (app.exitCode !== null)
      throw Error("Isolated app exited before WebView was ready");
    await pause(500);
  }
  assert.ok(page, "Debug WebView CDP unavailable");
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, j) => {
    socket.onopen = r;
    socket.onerror = j;
  });
  for (let i = 0; i < 60; i++) {
    if (await evaluate("Boolean(window.__TAURI_INTERNALS__)")) return;
    await pause(250);
  }
  throw Error("Tauri IPC unavailable");
}
let sequence = 0;
function cdpCall(method, params) {
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      socket.removeEventListener("message", receive);
      reject(Error("CDP timeout"));
    }, 50000);
    const receive = (event) => {
      const message = JSON.parse(event.data);
      if (message.id !== id) return;
      clearTimeout(timeout);
      socket.removeEventListener("message", receive);
      if (message.error) reject(Error("CDP failed"));
      else resolve(message.result);
    };
    socket.addEventListener("message", receive);
    socket.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await cdpCall("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (result.exceptionDetails)
    throw Error("Native invocation failed (expression intentionally omitted)");
  return result.result.value;
}
async function invoke(command, args) {
  const response = await evaluate(
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)}).then(result=>({ok:true,result}),error=>({ok:false,message:typeof error==='string'?error:error.message}))`,
  );
  if (!response.ok) throw Error(response.message);
  return response.result;
}
const auth = (action, args = {}) => invoke("lingque_auth", { action, ...args });
const ui = (operation, args = {}) =>
  invoke("workbench_call", { operation, args });
async function stop() {
  socket?.close();
  socket = null;
  if (app && app.exitCode === null) {
    const child = app;
    const exited = new Promise((r) => child.once("exit", r));
    child.kill();
    await exited;
  }
  app = null;
  await pause(1000);
}
function mcp() {
  const child = spawn(join(install, "nuphus-workbench-mcp.exe"), ["serve"], {
    env,
    windowsHide: true,
    stdio: ["pipe", "pipe", "ignore"],
  });
  bridges.push(child);
  const lines = createInterface({ input: child.stdout });
  const pending = new Map();
  let id = 0;
  lines.on("line", (line) => {
    const value = JSON.parse(line);
    const entry = pending.get(value.id);
    if (!entry) return;
    pending.delete(value.id);
    clearTimeout(entry.timer);
    value.error
      ? entry.reject(Error("MCP request rejected"))
      : entry.resolve(value.result);
  });
  const rpc = (method, params) =>
    new Promise((resolve, reject) => {
      const key = ++id;
      const timer = setTimeout(() => {
        pending.delete(key);
        reject(Error("MCP timeout"));
      }, 40000);
      pending.set(key, { resolve, reject, timer });
      child.stdin.write(
        JSON.stringify({ jsonrpc: "2.0", id: key, method, params }) + "\n",
      );
    });
  return {
    async init() {
      await rpc("initialize", {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "lingque-auth-acceptance", version: "1" },
      });
      child.stdin.write(
        '{"jsonrpc":"2.0","method":"notifications/initialized"}\n',
      );
    },
    async call(name, args = {}) {
      const response = await rpc("tools/call", {
        name: name.replaceAll(".", "_"),
        arguments: args,
      });
      if (response.isError)
        throw Error(
          response.structuredContent?.error?.code || "MCP tool rejected",
        );
      return response.structuredContent.result;
    },
  };
}
async function waitAuthorized() {
  for (let i = 0; i < 60; i++) {
    const status = await auth("status");
    if (status.authorized) return status;
    await pause(500);
  }
  throw Error("Session restoration failed");
}
try {
  await start();
  assert.equal((await auth("status")).authorized, false);
  for (const command of [
    "workbench_call",
    "execute_tool",
    "wf_run",
    "workbench_generate",
  ])
    await assert.rejects(
      invoke(command, { operation: "project.list", args: {} }),
      /登录|授权/,
    );
  assert.equal(
    (
      await fetch(`http://127.0.0.1:${port}/api/v1/project.list`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: "{}",
      })
    ).status,
    401,
  );
  const before = mcp();
  await before.init();
  await assert.rejects(before.call("project.list"), /product_auth_required/);
  console.log("PASS: unauthenticated IPC/HTTP/MCP blocked");
  const initial = await auth("login", {
    tenantCode: tenant,
    username,
    password,
  });
  loggedIn = true;
  assert.equal(initial.authorized, true);
  assert.equal(initial.policy.product_code, "lingque");
  const unclaimed = await auth("unclaimed");
  await auth("claim", {
    expectedEpoch: initial.epoch,
    projectIds: unclaimed.map((p) => p.project_id),
  });
  const project_id = (await ui("project.list"))[0].project_id;
  assert.equal((await auth("verify")).authorized, true);
  const client = mcp();
  await client.init();
  assert.ok(
    (await client.call("project.list")).some(
      (p) => p.project_id === project_id,
    ),
  );
  const draft = await client.call("workflow.import", {
    project_id,
    name: "Auth acceptance",
    document: {
      id: "import",
      name: "Auth acceptance",
      status: "Draft",
      inputs: [],
      steps: [{ id: "wait", name: "Wait", do: { sleep: 0.05 } }],
    },
  });
  const published = await client.call("workflow.save", {
    project_id,
    workflow_id: draft.workflow_id,
    revision: draft.revision,
  });
  const run = await client.call("workflow.run", {
    project_id,
    version_id: published.version_id,
    request_id: "auth-test-once",
  });
  for (let i = 0; i < 50; i++) {
    if (
      (await client.call("run.get", { project_id, run_id: run.run_id }))
        .status === "completed"
    )
      break;
    await pause(100);
  }
  assert.equal(
    (await client.call("run.get", { project_id, run_id: run.run_id })).status,
    "completed",
  );
  const device = await readFile(
    join(profile, "lingque-auth/device-id"),
    "utf8",
  );
  const sealed = await readFile(
    join(profile, "lingque-auth/session.sealed"),
    "utf8",
  );
  assert.ok(sealed.startsWith("enc:v1:"));
  assert.ok(!sealed.includes(password));
  await pause(500);
  const screenshot = await cdpCall("Page.captureScreenshot", { format: "png" });
  await writeFile(
    join(root, "authorized.png"),
    Buffer.from(screenshot.data, "base64"),
  );
  await stop();
  await start();
  const restored = await waitAuthorized();
  assert.equal(restored.epoch, initial.epoch);
  assert.equal(
    await readFile(join(profile, "lingque-auth/device-id"), "utf8"),
    device,
  );
  assert.equal(
    (
      await client.call("workflow.run", {
        project_id,
        version_id: published.version_id,
        request_id: "auth-test-once",
      })
    ).run_id,
    run.run_id,
  );
  console.log(
    "PASS: native login, encrypted persistence, heartbeat, MCP workflow, restart restoration, no replay",
  );
  const longDraft = await ui("workflow.import", {
    project_id,
    name: "Cancellation acceptance",
    document: {
      id: "import",
      name: "Cancellation acceptance",
      status: "Draft",
      inputs: [],
      steps: [{ id: "wait", name: "Long wait", do: { sleep: 60 } }],
    },
  });
  const longVersion = await ui("workflow.save", {
    project_id,
    workflow_id: longDraft.workflow_id,
    revision: longDraft.revision,
  });
  const longRun = await ui("workflow.run", {
    project_id,
    version_id: longVersion.version_id,
    request_id: "auth-cancel",
  });
  assert.equal((await auth("logout")).authorized, false);
  loggedIn = false;
  await assert.rejects(client.call("project.list"));
  const second = await auth("login", {
    tenantCode: tenant,
    username,
    password,
  });
  loggedIn = true;
  assert.notEqual(second.epoch, initial.epoch);
  await assert.rejects(client.call("project.list"));
  for (let i = 0; i < 30; i++) {
    if (
      (await ui("run.get", { project_id, run_id: longRun.run_id })).status ===
      "cancelled"
    )
      break;
    await pause(100);
  }
  assert.equal(
    (await ui("run.get", { project_id, run_id: longRun.run_id })).status,
    "cancelled",
  );
  assert.equal(
    await readFile(join(profile, "lingque-auth/device-id"), "utf8"),
    device,
  );
  await auth("logout");
  loggedIn = false;
  await stop();
  await start();
  assert.equal((await auth("status")).authorized, false);
  console.log(
    "PASS: logout stops running workflow, old MCP session rejected, same device preserved, logout survives restart",
  );
  console.log(JSON.stringify({ result: "PASS", evidence: root }));
} finally {
  if (loggedIn && socket) {
    try {
      await auth("logout");
    } catch {
      /* Local test profile retained for cleanup. */
    }
  }
  for (const bridge of bridges) {
    bridge.stdin.end();
    bridge.kill();
  }
  await stop();
}
