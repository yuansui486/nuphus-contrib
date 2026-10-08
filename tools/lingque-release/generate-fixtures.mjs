// Offline fixtures only. The ephemeral key never touches disk or production secrets.
import {
  generateKeyPairSync,
  randomBytes,
  sign,
  createHash,
} from "node:crypto";
import { writeFile } from "node:fs/promises";
import { gzipSync } from "node:zlib";
const directory = new URL(
  "../../src-tauri/src/workbench/updates/fixtures/",
  import.meta.url,
);
const { privateKey, publicKey } = generateKeyPairSync("ed25519");
const id = randomBytes(8);
const key = Buffer.concat([
  Buffer.from("Ed"),
  id,
  publicKey.export({ format: "der", type: "spki" }).subarray(-32),
]);
const encodedKey = Buffer.from(
  `untrusted comment: Lingque TEST public key\n${key.toString("base64")}\n`,
).toString("base64");
async function fixture(name, bytes) {
  const signature = sign(
    null,
    createHash("blake2b512").update(bytes).digest(),
    privateKey,
  );
  const comment = "Lingque offline test fixture";
  const global = sign(
    null,
    Buffer.concat([signature, Buffer.from(comment)]),
    privateKey,
  );
  const text = `untrusted comment: TEST signature\n${Buffer.concat([Buffer.from("ED"), id, signature]).toString("base64")}\ntrusted comment: ${comment}\n${global.toString("base64")}\n`;
  await writeFile(new URL(name, directory), bytes);
  await writeFile(
    new URL(`${name}.sig`, directory),
    Buffer.from(text).toString("base64") + "\n",
  );
}
function entry(name, value, mode) {
  const body = Buffer.from(value),
    header = Buffer.alloc(512);
  header.write(name, 0, 100);
  for (const [offset, width, value] of [
    [100, 8, mode],
    [108, 8, 0],
    [116, 8, 0],
    [124, 12, body.length],
    [136, 12, 0],
  ])
    header.write(
      value.toString(8).padStart(width - 1, "0") + "\0",
      offset,
      width,
    );
  header.fill(32, 148, 156);
  header.write("0", 156);
  header.write("ustar\0", 257);
  header.write("00", 263);
  header.write(
    [...header]
      .reduce((a, b) => a + b, 0)
      .toString(8)
      .padStart(6, "0") + "\0 ",
    148,
    8,
  );
  return Buffer.concat([
    header,
    body,
    Buffer.alloc((512 - (body.length % 512)) % 512),
  ]);
}
await writeFile(new URL("public.key", directory), encodedKey + "\n");
await fixture(
  "package.txt",
  Buffer.from("Lingque signed offline fixture v1\n"),
);
await fixture(
  "mac-fixture.app.tar.gz",
  gzipSync(
    Buffer.concat([
      entry(
        "LingqueFixture.app/Contents/Info.plist",
        '<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>lingque-fixture</string><key>CFBundleIdentifier</key><string>io.github.lingque.fixture</string><key>CFBundleShortVersionString</key><string>9.0.0</string></dict></plist>',
        0o644,
      ),
      entry(
        "LingqueFixture.app/Contents/MacOS/lingque-fixture",
        "#!/bin/sh\nprintf 'LINGQUE_UPDATED_9.0.0\\n'\n",
        0o755,
      ),
      entry(
        "LingqueFixture.app/Contents/Resources/version.txt",
        "9.0.0",
        0o644,
      ),
      Buffer.alloc(1024),
    ]),
  ),
);
console.log("已生成灵雀离线签名夹具；未使用或保存生产私钥。");
