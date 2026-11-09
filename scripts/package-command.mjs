export function pnpmScriptCommand(script, {
  platform = process.platform,
  env = process.env,
} = {}) {
  if (platform === "win32") {
    return {
      command: env.ComSpec || env.COMSPEC || "cmd.exe",
      args: ["/d", "/s", "/c", `pnpm.cmd run ${script}`],
    };
  }

  return { command: "pnpm", args: ["run", script] };
}
