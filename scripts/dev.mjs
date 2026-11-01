import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { runAsk, runWorker, worktreeRoot } from "./development/runtime.mjs";

export async function main(args) {
  const operation = args.shift();
  const rootIndex = args.indexOf("--worktree");
  const rootArgument = rootIndex < 0 ? path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..") : args[rootIndex + 1];
  if (!rootArgument) throw new Error("--worktree requires an absolute path");
  if (rootIndex >= 0) args.splice(rootIndex, 2);
  const root = worktreeRoot(rootArgument);
  if (operation === "mcp") {
    if (args.length) throw new Error("unexpected project MCP arguments");
    const { startMcp } = await import("./development/mcp.mjs");
    startMcp(root);
    return;
  }
  if (operation === "ask") {
    const input = { worktree: root };
    for (const [flag, key] of [["--delivery", "delivery"], ["--resume", "resume"]]) {
      const index = args.indexOf(flag);
      if (index >= 0) {
        if (!args[index + 1]) throw new Error(`${flag} requires a value`);
        input[key] = args[index + 1]; args.splice(index, 2);
      }
    }
    if (args[0] === "--") args.shift();
    if (args.length > 1) throw new Error("CLI verification takes one message; use project dev_ask for structured questions");
    input.message = args[0];
    const controller = new AbortController();
    const cancel = () => controller.abort();
    process.once("SIGINT", cancel); process.once("SIGTERM", cancel);
    let result;
    try { result = await runAsk(root, input, controller.signal); }
    finally { process.removeListener("SIGINT", cancel); process.removeListener("SIGTERM", cancel); }
    process.stdout.write(result.stdout);
    process.stderr.write(result.stderr);
    process.exitCode = result.code ?? 1;
    return;
  }
  if (["ui", "session", "status"].includes(operation)) {
    const native = operation === "ui" ? ["dev", "ui", "--worktree", root]
      : operation === "session" ? ["dev", "session", ...args, "--worktree", root]
      : ["dev", "status"];
    const result = await runWorker(root, native, { inheritStdio: operation === "session" && args.includes("--stdio") });
    process.stdout.write(result.stdout); process.stderr.write(result.stderr);
    process.exitCode = result.code ?? 1;
    return;
  }
  throw new Error("Use dev.mjs mcp|ask|ui|session|status [--worktree ABSOLUTE_PATH]");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).catch((error) => { console.error(error.message); process.exitCode = 1; });
}
