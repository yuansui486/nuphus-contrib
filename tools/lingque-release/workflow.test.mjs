import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

test("Windows MCP 和安装验收独立执行，不因空 LASTEXITCODE 提前成功退出", async () => {
  const workflow = await readFile(
    new URL("../../.github/workflows/workbench-release.yml", import.meta.url),
    "utf8",
  );
  const steps = workflow.split(/^      - /m);
  const mcp = steps.find((step) =>
    step.includes("./scripts/workbench-mcp-focus.ps1"),
  );
  const installer = steps.find((step) =>
    step.includes("-RequireTauriTemplate"),
  );
  const receipt = steps.find((step) =>
    step.includes("node tools/lingque-release/receipt.mjs"),
  );
  assert.ok(mcp && installer && receipt);
  assert.notEqual(mcp, installer, "安装验收必须是独立的 CI 步骤");
  assert.doesNotMatch(mcp, /\$LASTEXITCODE/);
  assert.match(installer, /cargo build[^\n]+--example lingque-updater-probe/);
  assert.match(installer, /-File src-tauri\/installer\/tests\/run\.ps1/);
  assert.match(
    installer,
    /if \(\$LASTEXITCODE -ne 0\) \{ exit \$LASTEXITCODE \}/,
  );
  assert.ok(steps.indexOf(installer) > steps.indexOf(mcp));
  assert.ok(steps.indexOf(receipt) > steps.indexOf(installer));
});
