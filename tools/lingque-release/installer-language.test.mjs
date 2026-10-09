import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

test(
  "NSIS 验收读取实际模板语言，兼容英文和中文并拒绝缺失文件",
  {
    skip: process.platform !== "win32",
  },
  async () => {
    const root = await mkdtemp(join(tmpdir(), "lingque-language-"));
    const script = fileURLToPath(
      new URL(
        "../../src-tauri/installer/tests/tauri-language.ps1",
        import.meta.url,
      ),
    );
    const select = () =>
      execFileSync(
        "powershell.exe",
        [
          "-NoProfile",
          "-NonInteractive",
          "-ExecutionPolicy",
          "Bypass",
          "-File",
          script,
          "-TauriNsisDirectory",
          root,
          "-NsisDirectory",
          root,
        ],
        { encoding: "utf8", stdio: "pipe" },
      ).trim();
    try {
      const languages = join(root, "Contrib", "Language files");
      await mkdir(languages, { recursive: true });
      await writeFile(join(root, "utils.nsh"), "");
      for (const language of ["English", "SimpChinese"]) {
        await writeFile(join(root, `${language}.nsh`), "");
        await writeFile(join(languages, `${language}.nlf`), "");
        await writeFile(
          join(root, "installer.nsi"),
          `; !insertmacro MUI_LANGUAGE "Stale"\r\n!insertmacro MUI_LANGUAGE "${language}"\r\n`,
        );
        assert.equal(select(), language);
      }
      // Both language files exist: the configured order, not stale files, wins.
      await writeFile(
        join(root, "installer.nsi"),
        '!insertmacro MUI_LANGUAGE "English"\n!insertmacro MUI_LANGUAGE "SimpChinese"\n',
      );
      assert.equal(select(), "English");
      await rm(join(root, "English.nsh"));
      assert.throws(select, /Missing generated Tauri language dependency/);
      await writeFile(join(root, "English.nsh"), "");
      await rm(join(languages, "English.nlf"));
      assert.throws(select, /Missing generated Tauri language dependency/);
      await writeFile(
        join(root, "installer.nsi"),
        '; !insertmacro MUI_LANGUAGE "English"\n',
      );
      assert.throws(select, /no supported MUI_LANGUAGE declaration/);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  },
);
