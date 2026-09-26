// Shared by local development and release packaging. Generated binaries are ignored.
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const root = fileURLToPath(new URL("..", import.meta.url));
const debug =
  process.argv.includes("--debug") || process.env.TAURI_ENV_DEBUG === "true";
const rust = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
if (rust.status !== 0)
  throw Error("rustc is required to build the MCP executable");
const host = rust.stdout.match(/^host: (.+)$/m)?.[1];
const target = process.env.TAURI_ENV_TARGET_TRIPLE || host;
if (!target || !/^[a-zA-Z0-9_-]+$/.test(target))
  throw Error("Invalid Rust target");
const args = [
  "build",
  "--locked",
  "-p",
  "nuphus-workbench",
  "--features",
  "gateway",
  "--bin",
  "nuphus-workbench-mcp",
];
if (!debug) args.push("--release");
if (target !== host) args.push("--target", target);
const build = spawnSync("cargo", args, { cwd: root, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status || 1);
const suffix = target.includes("windows") ? ".exe" : "";
const output = resolve(
  root,
  process.env.CARGO_TARGET_DIR || "target",
  ...(target !== host ? [target] : []),
  debug ? "debug" : "release",
  `nuphus-workbench-mcp${suffix}`,
);
const destination = resolve(
  root,
  "src-tauri/binaries",
  `nuphus-workbench-mcp-${target}${suffix}`,
);
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(output, destination);
console.log(`Prepared MCP executable: ${destination}`);
