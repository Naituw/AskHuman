import { effectScope, nextTick, ref } from "vue";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentKind, NewTaskOptions } from "../../lib/types";
import { useTaskSettings } from "./useTaskSettings";

const api = vi.hoisted(() => ({ options: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ newTaskOptions: api.options }));
const scopes: ReturnType<typeof effectScope>[] = [];
function catalog(patch: Partial<NewTaskOptions> = {}): NewTaskOptions {
  return {
    listed: true, speedControl: true, maxControl: false,
    models: [
      { id: "fast", label: "Fast model", efforts: ["low", "high"], defaultEffort: "low", fastSupported: true },
      { id: "slow", label: "Other", efforts: ["medium"], fastSupported: false },
      { id: "also-fast", label: "Also fast", efforts: ["high"], fastSupported: true },
    ],
    default: { label: "Fast model", efforts: ["low", "high"], defaultEffort: "high" }, ...patch,
  };
}
async function composer(kind: AgentKind = "codex") {
  const agent = ref<AgentKind | null>(null);
  const scope = effectScope(); scopes.push(scope);
  const state = scope.run(() => useTaskSettings(agent))!;
  agent.value = kind;
  await flushPromises();
  return { state, agent };
}
beforeEach(() => { localStorage.clear(); api.options.mockReset().mockResolvedValue(catalog()); });
afterEach(() => { for (const scope of scopes.splice(0)) scope.stop(); });

it("starts at default with explicit standard speed and cannot infer Fast from a default label", async () => {
  const { state } = await composer();
  expect(state.modelId.value).toBeNull();
  expect(state.speed.value).toBe("standard");
  state.setFast(true);
  expect(state.fast.value).toBe(false);
  expect(state.effortChoices.value).toEqual(["low", "high"]);
});

it("keeps Fast across supported models and resets it with a notice for an unsupported one", async () => {
  const { state } = await composer();
  state.selectModel("fast"); state.selectEffort("high"); state.setFast(true);
  state.selectModel("also-fast");
  expect(state.speed.value).toBe("fast"); expect(state.effortId.value).toBe("high");
  state.selectModel("slow");
  expect(state.effortId.value).toBeNull();
  expect(state.speed.value).toBe("standard"); expect(state.fastWasReset.value).toBe(true);
});

it("saves only submitted model / effort choices, including a failed submission, independently per Agent", async () => {
  const { state, agent } = await composer();
  state.selectModel("fast"); state.selectEffort("high"); state.setFast(true);
  expect((await composer()).state.modelId.value).toBeNull();
  state.remember();
  const restored = (await composer()).state;
  expect(restored.modelId.value).toBe("fast"); expect(restored.effortId.value).toBe("high");
  expect(restored.speed.value).toBe("standard");
  agent.value = "claude"; await flushPromises();
  expect(state.modelId.value).toBeNull(); expect(state.speed.value).toBeUndefined();
  agent.value = "codex"; await flushPromises();
  expect(state.modelId.value).toBe("fast"); expect(state.fast.value).toBe(false);
});

it("does not erase an unavailable saved model unless the user makes a new choice", async () => {
  const first = (await composer()).state;
  first.selectModel("fast"); first.selectEffort("high"); first.remember();
  api.options.mockResolvedValue(catalog({ models: [catalog().models[1]] }));
  const fallback = (await composer()).state;
  expect(fallback.modelId.value).toBeNull(); fallback.remember();
  api.options.mockResolvedValue(catalog());
  expect((await composer()).state.modelId.value).toBe("fast");
  fallback.selectModel(null); fallback.remember();
  expect((await composer()).state.modelId.value).toBeNull();
});

it("preserves preferences when discovery is unavailable and removes only a stale effort", async () => {
  const first = (await composer()).state;
  first.selectModel("fast"); first.selectEffort("high"); first.remember();
  api.options.mockResolvedValue(catalog({ listed: false, models: [] }));
  (await composer()).state.remember();
  api.options.mockResolvedValue(catalog({ models: [{ ...catalog().models[0], efforts: ["low"] }] }));
  const restored = (await composer()).state;
  expect(restored.modelId.value).toBe("fast"); expect(restored.effortId.value).toBeNull();
});

it("refreshes capabilities without overwriting edits, and resets stale Fast and Max choices", async () => {
  const { state } = await composer();
  state.selectModel("fast"); state.setFast(true);
  api.options.mockResolvedValue(catalog({ speedControl: false }));
  await state.reload();
  expect(state.modelId.value).toBe("fast"); expect(state.speed.value).toBeUndefined();
  expect(state.fastWasReset.value).toBe(true);
  api.options.mockResolvedValue(catalog({ maxControl: true, default: { efforts: [], maxRequired: true } }));
  const cursor = await composer("cursor");
  expect(cursor.state.maxBlocked.value).toBe(true);
  cursor.state.allowMax.value = true;
  cursor.state.resetTask();
  expect(cursor.state.allowMax.value).toBe(false);
  api.options.mockResolvedValue(catalog({ maxControl: false })); await cursor.state.reload();
  expect(cursor.state.maxControl.value).toBe(false);
  expect(cursor.state.offersModels.value).toBe(true);
});

it("discards a response for a previously selected Agent", async () => {
  let release!: (value: NewTaskOptions) => void;
  api.options.mockImplementationOnce(() => new Promise<NewTaskOptions>(resolve => { release = resolve; }));
  const { state, agent } = await composer();
  expect(state.loading.value).toBe(true);
  agent.value = "pi"; await flushPromises();
  release(catalog({ models: [] })); await flushPromises(); await nextTick();
  expect(state.offersModels.value).toBe(true); expect(state.loading.value).toBe(false);
});
