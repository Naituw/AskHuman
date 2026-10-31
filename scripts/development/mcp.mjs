import fs from "node:fs";
import path from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { McpServer } from "@modelcontextprotocol/server";
import { serveStdio } from "@modelcontextprotocol/server/stdio";
import * as z from "zod/v4";
import { pathKey, physicalPath, readWorkspace, record, runAsk } from "./runtime.mjs";
import { guard, monitor } from "./caller.mjs";

const option = z.object({ text: z.string(), recommended: z.boolean().optional() });
const question = z.object({ question: z.string(), options: z.array(option).optional() });
const inputSchema = z.object({
  worktree: z.string().describe("Absolute worktree path; must match this project connection"),
  message: z.string().optional(),
  questions: z.array(question).min(1).optional(),
  files: z.array(z.string()).optional(),
  delivery: z.enum(["local", "remote", "channels"]).optional(),
  resume: z.string().optional(),
});

function images(text, directory) {
  const content = [];
  let collecting = false, total = 0;
  for (const line of text.split(/\r?\n/)) {
    if (line === "[files]") { collecting = true; continue; }
    if (!line.trim()) { collecting = false; continue; }
    if (!collecting) continue;
    const extension = path.extname(line).toLowerCase();
    const mimeType = { ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".webp": "image/webp", ".gif": "image/gif" }[extension];
    if (!mimeType) continue;
    try {
      const actual = fs.realpathSync(line);
      const relative = path.relative(physicalPath(directory), physicalPath(actual));
      const stat = fs.statSync(actual);
      if (!relative || relative.startsWith("..") || path.isAbsolute(relative) || !stat.isFile()
          || stat.size > 10 * 1024 * 1024 || total + stat.size > 20 * 1024 * 1024) continue;
      content.push({ type: "image", data: fs.readFileSync(actual).toString("base64"), mimeType });
      total += stat.size;
    } catch { /* The retained text still contains the attachment path. */ }
  }
  return content;
}

export function startMcp(root) {
  const hash = createHash("sha256");
  for (const name of ["scripts/dev.mjs", "scripts/development/runtime.mjs", "scripts/development/mcp.mjs", "scripts/development/caller.mjs"]) {
    // The source files live with this controller, even in the no-build catalog test.
    hash.update(name); hash.update(fs.readFileSync(new URL(`../../${name}`, import.meta.url)));
  }
  const fingerprint = hash.digest("hex");
  let observation;
  const observe = () => {
    if (!fs.existsSync(path.join(root, ".askhuman-dev/workspace.json"))) return;
    readWorkspace(root);
    const directory = path.join(root, ".askhuman-dev/mcp-connections");
    fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
    if (physicalPath(directory) !== pathKey(directory)) throw new Error("linked project connection state requires review");
    observation = path.join(directory, `${process.pid}.json`);
    const temporary = `${observation}.${randomUUID()}.tmp`;
    try {
      fs.writeFileSync(temporary, JSON.stringify({ pid: process.pid, fingerprint, worktree: root }), { mode: 0o600, flag: "wx" });
      fs.renameSync(temporary, observation);
    } finally { try { fs.unlinkSync(temporary); } catch { /* Published or already removed. */ } }
  };
  process.once("exit", () => { if (observation) { try { fs.unlinkSync(observation); } catch { /* Already cleaned. */ } } });
  return serveStdio(() => {
    const server = new McpServer({ name: "askhuman-dev", version: "1.0.0" }, {
      instructions: "This connection belongs only to the AskHuman development worktree. Ordinary discussion and end-of-task handoff continue through the normal AskHuman connection. For explicit verification after installing this worktree, call this connection's dev_ask tool and wait for its answer. This project-only verification exception does not transfer daily native session credentials, permissions, Stop hooks, history or todos to Dev.",
    });
    server.registerTool("dev_ask", {
      description: `Verify the actual installed AskHuman development build at ${root}, then return the human's answer here. This tool is only for explicit development verification; ordinary discussion uses the normal AskHuman connection. Default delivery uses the formal remote relay. Resume retrieves an existing result without asking again.`,
      inputSchema,
      annotations: { destructiveHint: false, openWorldHint: false },
    }, async (input, context) => {
      const controller = new AbortController();
      const cancel = () => controller.abort();
      context.mcpReq.signal.addEventListener("abort", cancel, { once: true });
      if (context.mcpReq.signal.aborted) controller.abort();
      let stopMonitor = () => {};
      try {
        guard(context.mcpReq._meta);
        observe();
        stopMonitor = monitor(context.mcpReq._meta, controller);
        const result = await runAsk(root, input, controller.signal);
        if (result.code !== 0 || !result.stdout.trim()) {
          return { isError: true, content: [{ type: "text", text: `Dev verification ${result.id} failed (exit ${result.code}): ${result.stderr.trim()}` }] };
        }
        const content = [{ type: "text", text: result.stdout.trim() }];
        try {
          const saved = record(root, result.id);
          content.push(...images(result.stdout, saved.directory));
          content.push({ type: "text", text: `Dev verification metadata: ${JSON.stringify(saved.value.info)}` });
        } catch (error) {
          return { isError: true, content: [{ type: "text", text: `Dev answered, but its retained verification record could not be validated: ${error.message}` }] };
        }
        return { content };
      } catch (error) {
        return { isError: true, content: [{ type: "text", text: error.message }] };
      } finally {
        stopMonitor();
        context.mcpReq.signal.removeEventListener("abort", cancel);
      }
    });
    return server;
  });
}
