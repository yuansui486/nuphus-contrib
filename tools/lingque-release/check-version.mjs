import { readFile } from "node:fs/promises";
import { stableVersion, baseUrl } from "./manifest.mjs";
const config = JSON.parse(
  await readFile("src-tauri/tauri.workbench.conf.json", "utf8"),
);
const version = stableVersion(config.version);
const tag = process.argv[2] || process.env.RELEASE_TAG;
if (tag && tag !== `lingque-v${version}`)
  throw Error("灵雀标签与有效 Tauri 版本不一致");
if (
  config.identifier !== "io.github.yuansui486.nuphusworkbench" ||
  config.mainBinaryName !== "nuphus-workbench"
)
  throw Error("不允许改变现有灵雀安装身份");
if (
  !config.bundle.createUpdaterArtifacts ||
  !config.plugins.updater.pubkey ||
  JSON.stringify(config.plugins.updater.endpoints) !==
    JSON.stringify([`${baseUrl}/stable/latest.json`])
)
  throw Error("灵雀更新产物、公钥或独立更新源配置不正确");
const cargo = await readFile("Cargo.lock", "utf8");
const lock = JSON.parse(await readFile("frontend/package-lock.json", "utf8"));
for (const [native, frontend] of [
  ["tauri", "@tauri-apps/api"],
  ["tauri-plugin-updater", "@tauri-apps/plugin-updater"],
]) {
  const rust = new RegExp(
    `\\[\\[package\\]\\]\\s+name = "${native}"\\s+version = "([^"]+)"`,
  ).exec(cargo)?.[1];
  const js = lock.packages[`node_modules/${frontend}`]?.version;
  if (
    !rust ||
    !js ||
    rust.split(".").slice(0, 2).join(".") !==
      js.split(".").slice(0, 2).join(".")
  )
    throw Error(`Rust 与前端 API 主次版本不一致：${native} ${rust} / ${js}`);
}
await readFile(`docs/lingque-releases/${version}.md`, "utf8");
console.log(`灵雀 ${version} 有效配置和发布版本检查通过`);
