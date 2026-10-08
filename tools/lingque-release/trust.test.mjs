import { test } from "node:test";
import assert from "node:assert/strict";
import { validateBuildRun, validateReceipt } from "./trust.mjs";
test("复用只能来自同仓库已成功验收且未过期的发布流程", () => {
  const run = {
    repository: { full_name: "owner/repo" },
    head_repository: { full_name: "owner/repo" },
    path: ".github/workflows/workbench-dispatch.yml",
    event: "workflow_dispatch",
    status: "completed",
    conclusion: "success",
    created_at: new Date().toISOString(),
  };
  validateBuildRun(run, "owner/repo");
  for (const changed of [
    { conclusion: "failure" },
    { event: "pull_request" },
    { path: "evil.yml" },
    { created_at: "2020-01-01" },
    { repository: { full_name: "other/repo" } },
  ])
    assert.throws(() => validateBuildRun({ ...run, ...changed }, "owner/repo"));
});
test("回执核对真实产品 SHA，而不是 dispatcher 的默认分支 SHA", () => {
  const expected = {
    sha: "product-sha",
    version: "0.1.1",
    platform: "windows-x64",
    run: "42",
    repository: "owner/repo",
  };
  const receipt = {
    ...expected,
    schema: 1,
    accepted: true,
    files: { "nsis/package.exe": "hash", "nsis/package.exe.sig": "hash" },
  };
  validateReceipt(receipt, expected);
  for (const changed of [
    { sha: "dispatcher-sha" },
    { accepted: false },
    { run: "43" },
    { platform: "macos-arm64" },
  ])
    assert.throws(() => validateReceipt({ ...receipt, ...changed }, expected));
});
