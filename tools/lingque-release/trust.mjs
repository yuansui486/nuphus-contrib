export function validateBuildRun(run, repository, now = Date.now()) {
  if (
    run.repository?.full_name !== repository ||
    run.head_repository?.full_name !== repository ||
    run.event !== "workflow_dispatch" ||
    run.status !== "completed" ||
    run.conclusion !== "success" ||
    ![
      ".github/workflows/workbench-release.yml",
      ".github/workflows/workbench-dispatch.yml",
    ].includes(run.path) ||
    !Number.isFinite(Date.parse(run.created_at)) ||
    now - Date.parse(run.created_at) > 13 * 86400000
  )
    throw Error("仅可复用同仓库、同发布流程、13 天内完整验收通过的手动构建");
  // A dispatcher head SHA is NOT the product source SHA. Receipts from both
  // accepted platform jobs bind the actual checked-out source and artifact bytes.
}
export function validateReceipt(
  receipt,
  { sha, version, platform, run, repository },
) {
  if (
    receipt.schema !== 1 ||
    receipt.accepted !== true ||
    receipt.sha !== sha ||
    receipt.version !== version ||
    receipt.platform !== platform ||
    receipt.run !== String(run) ||
    receipt.repository !== repository ||
    !receipt.files ||
    Object.keys(receipt.files).length < 2
  )
    throw Error("构建验收回执与发布提交、版本、平台或运行来源不一致");
}
