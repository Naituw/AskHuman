// Generated and managed by AskHuman. Changes inside this file will be replaced.
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";

const CONFIG = /* ASKHUMAN_CONFIG_BEGIN */ __ASKHUMAN_CONFIG_JSON__ /* ASKHUMAN_CONFIG_END */;
const ASK_TIMEOUT_MS = 24 * 60 * 60 * 1000;
const SHORT_TIMEOUT_MS = 30 * 1000;
const RECOVERY_PROMPT =
  "[ASKHUMAN CONTEXT RECOVERY] Continue following the AskHuman mandatory interaction protocol from AGENTS.md. Ask questions only through the AskHuman CLI. If the exact prior AskHuman question or answer is uncertain after compaction, run AskHuman --show-last before continuing.";

type JsonObject = Record<string, unknown>;
type RunningCommand = {
  child: ChildProcessWithoutNullStreams;
  result: Promise<JsonObject | undefined>;
};

let generation = 0;
let recoveryPending = false;
let controlGeneration = randomUUID();
let controlTimer: ReturnType<typeof setTimeout> | undefined;
let controlPoll: RunningCommand | undefined;
let pendingControlId: string | undefined;
let activeReportTurnId: string | undefined;
let pendingContinuationTurnId: string | undefined;
let lastRun:
  | { generation: number; stopReason?: string; message?: string }
  | undefined;
let pendingStop:
  | { generation: number; child: ChildProcessWithoutNullStreams }
  | undefined;

function sessionPayload(ctx: ExtensionContext, extra: JsonObject = {}): JsonObject {
  const manager = ctx.sessionManager;
  const sessionId = process.env.PI_SESSION_ID || manager.getSessionId();
  const sessionFile = process.env.PI_SESSION_FILE || manager.getSessionFile();
  return {
    session_id: sessionId,
    transcript_path: sessionFile ?? null,
    cwd: ctx.cwd,
    ...extra,
  };
}

function commandEnv(ctx: ExtensionContext): NodeJS.ProcessEnv {
  const payload = sessionPayload(ctx);
  return {
    ...process.env,
    PI_CODING_AGENT: "true",
    PI_SESSION_ID: String(payload.session_id ?? ""),
    PI_SESSION_FILE: String(payload.transcript_path ?? ""),
  };
}

function spawnJson(
  args: string[],
  payload: JsonObject,
  ctx: ExtensionContext,
  timeoutMs: number,
): RunningCommand {
  const child = spawn(CONFIG.executable, args, {
    cwd: ctx.cwd,
    env: commandEnv(ctx),
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  const result = new Promise<JsonObject | undefined>((resolve) => {
    let stdout = "";
    let settled = false;
    const finish = (value?: JsonObject) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(value);
    };
    const timer = setTimeout(() => {
      child.kill();
      finish();
    }, timeoutMs);
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      if (stdout.length < 1024 * 1024) stdout += chunk;
    });
    child.stderr.resume();
    child.on("error", () => finish());
    child.on("close", () => {
      const trimmed = stdout.trim();
      if (!trimmed) return finish();
      try {
        const parsed = JSON.parse(trimmed);
        finish(parsed && typeof parsed === "object" ? parsed : undefined);
      } catch {
        finish();
      }
    });
  });
  child.stdin.on("error", () => {});
  child.stdin.end(JSON.stringify(payload));
  return { child, result };
}

async function reportNative(pi: ExtensionAPI, ctx: ExtensionContext): Promise<void> {
  if (!CONFIG.cli && !CONFIG.lifecycle) return;
  const usage = typeof ctx.getContextUsage === "function" ? ctx.getContextUsage() : undefined;
  await spawnJson(["__session-info", "pi"], sessionPayload(ctx, {
    native_context: {
      model: ctx.model?.id || ctx.model?.name || null,
      reasoning: pi.getThinkingLevel(),
      usage: usage ?? null,
    },
  }), ctx, SHORT_TIMEOUT_MS).result;
}

function report(ctx: ExtensionContext, event: string, extra: JsonObject = {}): void {
  const command = spawnJson(
    ["__agent-hook", "pi", event],
    sessionPayload(ctx, {
      ...(["turn-start", "activity", "turn-end"].includes(event)
        ? { turn_id: activeReportTurnId ?? `${controlGeneration}:${generation}` } : {}),
      ...extra,
    }),
    ctx,
    SHORT_TIMEOUT_MS,
  );
  void command.result;
}

function stopControlPolling(): void {
  if (controlTimer) clearTimeout(controlTimer);
  controlTimer = undefined;
  controlPoll?.child.kill();
  controlPoll = undefined;
  pendingControlId = undefined;
}

function startControlPolling(pi: ExtensionAPI, ctx: ExtensionContext): void {
  stopControlPolling();
  if (!CONFIG.lifecycle) return;
  const lease = controlGeneration;
  const signal = (extra: JsonObject = {}) => sessionPayload(ctx, {
    generation: lease, pid: process.pid, idle: ctx.isIdle() && !ctx.hasPendingMessages(), ...extra,
  });
  const tick = async () => {
    if (lease !== controlGeneration) return;
    try {
      controlPoll = spawnJson(["__agent-control"], signal(), ctx, 4000);
      const reply = await controlPoll.result;
      controlPoll = undefined;
      if (lease !== controlGeneration) return;
      const operation = reply?.operation as JsonObject | undefined;
      if (operation && typeof operation.id === "string" && typeof operation.text === "string") {
        const id = operation.id;
        let status = "rejected";
        if (ctx.isIdle() && !ctx.hasPendingMessages()) {
          pendingControlId = id;
          const files = Array.isArray(operation.files) ? operation.files as JsonObject[] : [];
          const attachments = files.filter(file => typeof file.path === "string")
            .map(file => `- ${String(file.name ?? "attachment")}: ${String(file.path)}`).join("\n");
          const text = operation.text + (attachments ? `\n\nUser attachments (read the files as needed):\n${attachments}` : "");
          try {
            await pi.sendUserMessage(text);
            status = "accepted";
            if (pendingControlId === id) pendingControlId = undefined;
          } catch {
            // The native call might have crossed its acceptance boundary before rejecting.
            status = "unknown";
            if (pendingControlId === id) pendingControlId = undefined;
          }
        }
        if (lease === controlGeneration) {
          await spawnJson(["__agent-control"], signal({ operationId: id, status }), ctx, 4000).result;
        }
      }
    } catch {
      // A disconnected daemon or disposed session must not affect the native agent.
    } finally {
      if (lease === controlGeneration) controlTimer = setTimeout(() => { void tick(); }, 1500);
    }
  };
  controlTimer = setTimeout(() => { void tick(); }, 250);
}

function shellTokens(segment: string): string[] {
  const tokens: string[] = [];
  const pattern = /"([^"]*)"|'([^']*)'|([^\s]+)/g;
  for (const match of segment.matchAll(pattern)) {
    tokens.push(match[1] ?? match[2] ?? match[3] ?? "");
  }
  return tokens;
}

function basename(value: string): string {
  const unquoted = value.replace(/^['"]|['"]$/g, "");
  const name = unquoted.split(/[\\/]/).pop()?.toLowerCase() ?? "";
  return name.endsWith(".exe") ? name.slice(0, -4) : name;
}

function isAskHumanCommand(command: string): boolean {
  const allowed = new Set([
    "askhuman",
    "humaninloop",
    basename(CONFIG.programName),
    basename(CONFIG.executable),
  ]);
  for (const segment of command.split(/&&|\|\||[;|\n]/)) {
    const tokens = shellTokens(segment.trim());
    let index = 0;
    while (index < tokens.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(tokens[index])) index += 1;
    if (["command", "exec"].includes(tokens[index]?.toLowerCase())) index += 1;
    if (tokens[index]?.toLowerCase() === "env") {
      index += 1;
      while (index < tokens.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(tokens[index])) index += 1;
    }
    if (allowed.has(basename(tokens[index] ?? ""))) return true;
  }
  return false;
}

function boundedInput(input: unknown): unknown {
  try {
    const encoded = JSON.stringify(input);
    return encoded.length <= 64 * 1024 ? input : {};
  } catch {
    return {};
  }
}

function assistantSummary(messages: unknown[]): { stopReason?: string; message?: string } {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index] as JsonObject;
    if (message?.role !== "assistant") continue;
    const content = Array.isArray(message.content) ? message.content : [];
    const text = content
      .filter((part): part is { type: "text"; text: string } =>
        Boolean(part && typeof part === "object" && (part as JsonObject).type === "text"),
      )
      .map((part) => part.text)
      .join("\n")
      .trim();
    return {
      stopReason: typeof message.stopReason === "string" ? message.stopReason : undefined,
      message: text || undefined,
    };
  }
  return {};
}

function cancelPendingStop(): void {
  const pending = pendingStop;
  pendingStop = undefined;
  if (pending && !pending.child.killed) pending.child.kill();
}

export default function askHumanExtension(pi: ExtensionAPI) {
  pi.on("model_select", async (_event, ctx) => { await reportNative(pi, ctx); });
  pi.on("thinking_level_select", async (_event, ctx) => { await reportNative(pi, ctx); });
  pi.on("session_start", async (_event, ctx) => {
    stopControlPolling();
    controlGeneration = randomUUID();
    activeReportTurnId = undefined;
    pendingContinuationTurnId = undefined;
    generation += 1;
    recoveryPending = false;
    lastRun = undefined;
    cancelPendingStop();
    await reportNative(pi, ctx);
    if (CONFIG.lifecycle) report(ctx, "session-start");
    startControlPolling(pi, ctx);
  });

  pi.on("before_agent_start", async (event) => {
    if (!CONFIG.cli || !recoveryPending) return;
    recoveryPending = false;
    return { systemPrompt: `${event.systemPrompt}\n\n${RECOVERY_PROMPT}` };
  });

  pi.on("session_compact", async (_event, ctx) => {
    await reportNative(pi, ctx);
    if (CONFIG.cli) recoveryPending = true;
  });

  pi.on("agent_start", async (_event, ctx) => {
    generation += 1;
    activeReportTurnId = pendingContinuationTurnId ?? `${controlGeneration}:${generation}`;
    pendingContinuationTurnId = undefined;
    const token = generation, operationId = pendingControlId;
    pendingControlId = undefined;
    lastRun = undefined;
    cancelPendingStop();
    await reportNative(pi, ctx);
    if (generation !== token) return;
    if (CONFIG.lifecycle) report(ctx, "turn-start", operationId ? { control_operation_id: operationId } : {});
  });

  pi.on("tool_call", async (event, ctx) => {
    await reportNative(pi, ctx);
    const input = event.input as JsonObject;
    if (
      CONFIG.cli &&
      event.toolName === "bash" &&
      typeof input.command === "string" &&
      isAskHumanCommand(input.command)
    ) {
      input.timeout = 86400;
    }
    if (!CONFIG.lifecycle) return;
    const command = spawnJson(
      ["__agent-hook", "pi", "activity"],
      sessionPayload(ctx, {
        hook_event_name: "PreToolUse",
        tool_name: event.toolName,
        tool_input: boundedInput(event.input),
      }),
      ctx,
      ASK_TIMEOUT_MS,
    );
    const decision = await command.result;
    if (decision?.block === true && typeof decision.reason === "string") {
      return { block: true, reason: decision.reason };
    }
  });

  pi.on("tool_result", async (event, ctx) => {
    if (!CONFIG.lifecycle) return;
    report(ctx, "activity", {
      hook_event_name: "PostToolUse",
      tool_name: event.toolName,
      tool_response: true,
    });
  });

  pi.on("agent_end", async (event) => {
    lastRun = { generation, ...assistantSummary(event.messages as unknown[]) };
  });

  pi.on("agent_settled", async (_event, ctx) => {
    const settledGeneration = generation, run = lastRun;
    await reportNative(pi, ctx);
    if (settledGeneration !== generation) return;
    if (!CONFIG.stop || !run || run.generation !== generation || run.stopReason !== "stop") {
      // Tell AskHuman why the turn ended so a failed run is not announced as finished.
      const reason = run && run.generation === generation ? run.stopReason : undefined;
      if (CONFIG.lifecycle) report(ctx, "turn-end", reason ? { stop_reason: reason } : {});
      return;
    }

    const token = generation, logicalTurnId = activeReportTurnId;
    cancelPendingStop();
    const command = spawnJson(
      ["__stop-hook", "pi", "confirm"],
      sessionPayload(ctx, {
        hook_event_name: "Stop",
        stop_reason: run.stopReason,
        last_assistant_message: run.message ?? "",
      }),
      ctx,
      ASK_TIMEOUT_MS,
    );
    pendingStop = { generation: token, child: command.child };
    void command.result.then(async (decision) => {
      if (
        !pendingStop ||
        pendingStop.child !== command.child ||
        pendingStop.generation !== token ||
        generation !== token
      ) {
        return;
      }
      pendingStop = undefined;
      const prompt = decision?.followup_message;
      if (
        typeof prompt === "string" &&
        prompt.trim() &&
        ctx.isIdle() &&
        !ctx.hasPendingMessages()
      ) {
        try {
          // A managed Stop continuation is still the same unreported logical round.
          // Ordinary later user messages always receive a fresh identity in agent_start.
          pendingContinuationTurnId = logicalTurnId;
          await pi.sendUserMessage(prompt);
          if (pendingContinuationTurnId === logicalTurnId) pendingContinuationTurnId = undefined;
          return;
        } catch {
          pendingContinuationTurnId = undefined;
          // Fail open and report the settled turn below.
        }
      }
      if (CONFIG.lifecycle) report(ctx, "turn-end", { stop_reason: run.stopReason });
    });
  });

  pi.on("session_shutdown", async (_event, ctx) => {
    stopControlPolling();
    controlGeneration = randomUUID();
    generation += 1;
    lastRun = undefined;
    cancelPendingStop();
    if (CONFIG.lifecycle) report(ctx, "session-end");
  });
}
