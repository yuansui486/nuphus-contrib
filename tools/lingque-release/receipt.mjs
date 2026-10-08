import { readFile, readdir, writeFile } from "node:fs/promises";
import { join, relative } from "node:path";
import { sha256 } from "./manifest.mjs";
import { validateReceipt } from "./trust.mjs";
async function files(root, directory = root) {
  const result = {};
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory() && !entry.name.endsWith(".app"))
      Object.assign(result, await files(root, path));
    else if (
      entry.isFile() &&
      /\.(exe|exe.sig|app.tar.gz|app.tar.gz.sig|dmg)$/.test(entry.name)
    )
      result[relative(root, path).replaceAll("\\", "/")] = sha256(
        await readFile(path),
      );
  }
  return Object.fromEntries(
    Object.entries(result).sort(([a], [b]) => a.localeCompare(b)),
  );
}
const source = process.argv[2];
if (source) {
  for (const platform of ["windows-x64", "macos-arm64"]) {
    const root = join(source, `lingque-${platform}`);
    const receipt = JSON.parse(
      await readFile(join(root, "build-receipt.json"), "utf8"),
    );
    validateReceipt(receipt, {
      sha: process.env.BUILD_SHA,
      version: process.env.BUILD_VERSION,
      platform,
      run: process.env.BUILD_RUN,
      repository: process.env.GITHUB_REPOSITORY,
    });
    if (JSON.stringify(receipt.files) !== JSON.stringify(await files(root)))
      throw Error("产物哈希与已通过验收的回执不同");
  }
} else {
  const root = "target/release/bundle";
  const receipt = {
    schema: 1,
    accepted: true,
    sha: process.env.BUILD_SHA,
    version: process.env.BUILD_VERSION,
    platform: process.env.BUILD_PLATFORM,
    run: process.env.GITHUB_RUN_ID,
    repository: process.env.GITHUB_REPOSITORY,
    files: await files(root),
  };
  await writeFile(
    join(root, "build-receipt.json"),
    JSON.stringify(receipt, null, 2) + "\n",
  );
}
