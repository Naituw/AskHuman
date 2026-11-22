#!/usr/bin/env node
// 版本号单一事实来源同步脚本。
// 用法: node scripts/bump-version.mjs <version>   (如 0.2.0 或 0.2.0-rc.1)
//
// 一次性写入以下位置，避免漂移：
//   - src-tauri/Cargo.toml            ([package] version)
//   - src-tauri/tauri.conf.json       (version)
//   - package.json                    (前端包 version)
//   - packaging/npm/humaninloop/package.json  (version + optionalDependencies 锁定)
//   - packaging/npm/platforms/*/package.json  (各平台子包 version)
// 注: AskHuman --version 文案取自 Cargo.toml 的 CARGO_PKG_VERSION，无需单独处理。

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(version)) {
  console.error(
    "用法: node scripts/bump-version.mjs <version>   (如 0.2.0 或 0.2.0-rc.1)"
  );
  process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function updateJson(relPath, mutate) {
  const p = join(root, relPath);
  const json = JSON.parse(readFileSync(p, "utf8"));
  mutate(json);
  writeFileSync(p, JSON.stringify(json, null, 2) + "\n");
  console.log(`updated ${relPath}`);
}

function updateCargoToml(relPath) {
  const p = join(root, relPath);
  const text = readFileSync(p, "utf8");
  // 仅替换 [package] 段内首个 version 行，避免误改依赖项的 version。
  const next = text.replace(
    /(\[package\][\s\S]*?\nversion\s*=\s*")[^"]*(")/,
    `$1${version}$2`
  );
  if (next === text) {
    console.error(`错误: 未能在 ${relPath} 的 [package] 段找到 version 行`);
    process.exit(1);
  }
  writeFileSync(p, next);
  console.log(`updated ${relPath}`);
}

const PLATFORM_PKGS = [
  "@humaninloop/darwin-arm64",
  "@humaninloop/darwin-x64",
  "@humaninloop/win32-x64",
  "@humaninloop/linux-x64",
];

updateCargoToml("src-tauri/Cargo.toml");

updateJson("src-tauri/tauri.conf.json", (j) => {
  j.version = version;
});

updateJson("package.json", (j) => {
  j.version = version;
});

updateJson("packaging/npm/humaninloop/package.json", (j) => {
  j.version = version;
  for (const name of PLATFORM_PKGS) {
    if (j.optionalDependencies && name in j.optionalDependencies) {
      j.optionalDependencies[name] = version;
    }
  }
});

updateJson("packaging/npm/platforms/darwin-arm64/package.json", (j) => {
  j.version = version;
});
updateJson("packaging/npm/platforms/darwin-x64/package.json", (j) => {
  j.version = version;
});
updateJson("packaging/npm/platforms/win32-x64/package.json", (j) => {
  j.version = version;
});
updateJson("packaging/npm/platforms/linux-x64/package.json", (j) => {
  j.version = version;
});

console.log(`\n版本已同步为 ${version}`);
console.log(
  `下一步: cargo update --workspace --offline（刷新 Cargo.lock 里的版本）→ 检查 diff → 提交并推送 → CI 全绿后打注解 tag v${version}，只推这一个 tag（不要 --tags）。见 docs/development.md 的 Release。`
);
