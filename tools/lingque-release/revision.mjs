import { execFileSync } from "node:child_process";
import { appendFile, readFile } from "node:fs/promises";
import { validateBuildRun } from "./trust.mjs";
const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();
const config = JSON.parse(
  await readFile("src-tauri/tauri.workbench.conf.json", "utf8"),
);
const sha = git("rev-parse", "HEAD");
const tag = process.env.RELEASE_TAG || `lingque-v${config.version}`;
execFileSync(
  process.execPath,
  ["tools/lingque-release/check-version.mjs", tag],
  { stdio: "inherit" },
);
if (process.env.PUBLISH === "true") {
  if (
    !process.env.RELEASE_TAG ||
    git("rev-parse", `refs/tags/${tag}^{commit}`) !== sha
  )
    throw Error("发布标签必须存在且指向本次构建提交");
}
if (process.env.REUSE_RUN) {
  if (!/^\d+$/.test(process.env.REUSE_RUN)) throw Error("构建运行 ID 无效");
  const result = execFileSync(
    "gh",
    [
      "api",
      `repos/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.REUSE_RUN}`,
    ],
    { encoding: "utf8" },
  );
  validateBuildRun(JSON.parse(result), process.env.GITHUB_REPOSITORY);
}
await appendFile(
  process.env.GITHUB_OUTPUT,
  `sha=${sha}\nversion=${config.version}\ndate=${git("show", "-s", "--format=%cI", sha)}\n`,
);
