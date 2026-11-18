#!/usr/bin/env node
// Prints the path of the executable that a `cargo build` reported for one binary target.
//
// install.sh builds with `--message-format=json-render-diagnostics`, keeps cargo's messages in a
// file and asks this script where the binary went. Taking the path from cargo, instead of
// rebuilding it from `CARGO_TARGET_DIR` or `src-tauri/target`, means the installer always installs
// the file this very build produced, wherever cargo put it. (A hard-coded path installed an older
// build without any error whenever the environment moved the target directory.)
//
// A build that found everything up to date reports its artifact as well (`"fresh": true`), so there
// is no modification-time comparison to make: cargo leaves the file untouched when nothing changed.
//
// Usage: node scripts/cargo-artifact.mjs MESSAGES_FILE BINARY_NAME

import fs from "node:fs";

const [messagesFile, name] = process.argv.slice(2);
if (!messagesFile || !name) {
  console.error("Usage: cargo-artifact.mjs MESSAGES_FILE BINARY_NAME");
  process.exit(2);
}

let executable = null;
for (const line of fs.readFileSync(messagesFile, "utf8").split("\n")) {
  // Cargo prints one JSON object per line; anything else (a stray build-script line) is not ours.
  if (!line.startsWith("{")) continue;
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    continue;
  }
  if (message?.reason !== "compiler-artifact") continue;
  const target = message.target;
  if (target?.name !== name || !Array.isArray(target.kind) || !target.kind.includes("bin")) continue;
  // The last report wins if a target is ever reported twice.
  if (typeof message.executable === "string" && message.executable !== "") executable = message.executable;
}

if (executable === null) {
  console.error(`错误: cargo 没有报告 ${name} 的可执行文件，无法确定要安装哪个产物`);
  process.exit(1);
}
if (!fs.statSync(executable, { throwIfNoEntry: false })?.isFile()) {
  console.error(`错误: cargo 报告的编译产物不存在: ${executable}`);
  process.exit(1);
}
console.log(executable);
