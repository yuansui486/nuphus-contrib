import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile, mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { prepare, sha256 } from "./manifest.mjs";
import { publish } from "./publish.mjs";
const fixture = new URL(
  "../../src-tauri/src/workbench/updates/fixtures/",
  import.meta.url,
);
const bytes = await readFile(new URL("package.txt", fixture));
const signature = await readFile(new URL("package.txt.sig", fixture), "utf8");
const key = await readFile(new URL("public.key", fixture), "utf8");
const stable = "12box/lingque/updates/stable/latest.json";
test("OSS 先上传并核对版本对象，再切清单；拒绝覆盖、降级及中途失败", async () => {
  const root = await mkdtemp(join(tmpdir(), "lingque-publisher-"));
  try {
    for (const [platform, suffixes] of [
      ["windows-x64", [".exe"]],
      ["macos-arm64", [".app.tar.gz", ".dmg"]],
    ]) {
      const directory = join(root, `lingque-${platform}`);
      await mkdir(directory);
      for (const suffix of suffixes) {
        const name = join(directory, `Lingque_0.1.1_${platform}${suffix}`);
        await writeFile(name, bytes);
        if (suffix !== ".dmg") await writeFile(`${name}.sig`, signature);
      }
    }
    const output = join(root, "prepared");
    await prepare({
      source: root,
      output,
      version: "0.1.1",
      publicKey: key,
      notes: "测试",
      date: "2026-10-08",
    });
    const objects = new Map(),
      writes = [];
    let failObject = false;
    const client = {
      head: async (name) => {
        if (!objects.has(name)) throw { status: 404 };
        return { res: { headers: objects.get(name).headers } };
      },
      get: async (name) => ({ content: objects.get(name).bytes }),
      put: async (name, source, options) => {
        if (failObject && name !== stable) throw Error("模拟上传失败");
        const value = Buffer.isBuffer(source) ? source : await readFile(source);
        writes.push(name);
        objects.set(name, {
          bytes: value,
          headers: {
            ...options.headers,
            "content-length": String(value.length),
            etag: "etag",
          },
        });
      },
    };
    const options = {
      client,
      releaseConfig: {
        version: "0.1.1",
        plugins: { updater: { pubkey: key } },
      },
      publicFetch: async () => ({
        ok: true,
        arrayBuffer: async () => objects.get(stable).bytes,
      }),
    };
    failObject = true;
    await assert.rejects(publish(output, options), /模拟上传失败/);
    assert.equal(objects.has(stable), false);
    failObject = false;
    await publish(output, options);
    assert.equal(writes.at(-1), stable);
    assert.equal(
      objects.get(stable).headers["Cache-Control"],
      "no-cache, max-age=0, must-revalidate",
    );
    assert(writes.every((name) => name.startsWith("12box/lingque/updates/")));
    const count = writes.length;
    await publish(output, options);
    assert.equal(writes.length, count + 1, "相同产物重试不覆盖 immutable 对象");
    assert.equal(objects.get(stable).headers["If-Match"], "etag");
    const first = writes[0];
    objects.get(first).headers["x-oss-meta-sha256"] = "different";
    await assert.rejects(publish(output, options), /禁止覆盖/);
    const newer = Buffer.from(JSON.stringify({ version: "0.1.2" }));
    objects.set(stable, {
      bytes: newer,
      headers: { "x-oss-meta-sha256": sha256(newer), etag: "newer" },
    });
    await assert.rejects(publish(output, options), /旧版本/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
