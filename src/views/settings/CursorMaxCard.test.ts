import { flushPromises, mount } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import CursorMaxCard from "./CursorMaxCard.vue";

const plan = vi.hoisted(() => vi.fn());
const useSettingsContext = vi.hoisted(() => vi.fn());

vi.mock("../../lib/ipc", () => ({ agentTaskCursorPlan: plan }));
vi.mock("./context", () => ({ useSettingsContext }));

const persist = vi.fn(async () => {});

function mountCard(cursorMax = { policy: "enforce-off", plan: "auto" }) {
  const config = ref({ agentTasks: reactive({ cursorMax: reactive(cursorMax) }) });
  useSettingsContext.mockReturnValue({ config, persist });
  return { config, wrapper: mount(CursorMaxCard, { global: { plugins: [i18n] } }) };
}

beforeEach(() => {
  persist.mockClear();
  plan.mockReset().mockResolvedValue("legacy");
  i18n.global.locale.value = "en";
});

it("shows the stored policy and plan and what Cursor's data says", async () => {
  const { wrapper } = mountCard();
  await flushPromises();
  expect((wrapper.get("[data-testid=cursor-max-policy]").element as HTMLSelectElement).value).toBe(
    "enforce-off",
  );
  expect((wrapper.get("[data-testid=cursor-max-plan]").element as HTMLSelectElement).value).toBe("auto");
  expect(wrapper.get("[data-testid=cursor-max-detected]").text()).toBe(
    "Detected: Old plan (request-based)",
  );
  expect(wrapper.text()).toContain("sticky");
});

it("saves a changed policy and explains what it does", async () => {
  const { config, wrapper } = mountCard();
  await flushPromises();
  await wrapper.get("[data-testid=cursor-max-policy]").setValue("inherit");
  expect(config.value.agentTasks.cursorMax.policy).toBe("inherit");
  expect(persist).toHaveBeenCalledTimes(1);
  expect(wrapper.text()).toContain("No intervention, no refusal, no notice");
});

it("saves a plan chosen by hand", async () => {
  const { config, wrapper } = mountCard();
  await flushPromises();
  await wrapper.get("[data-testid=cursor-max-plan]").setValue("token-based");
  expect(config.value.agentTasks.cursorMax.plan).toBe("token-based");
  expect(persist).toHaveBeenCalledTimes(1);
});

it.each([
  ["token-based", "Detected: New plan (no Max mode)"],
  ["unknown", "treated as the old plan"],
])("reports a %s detection", async (detected, text) => {
  plan.mockResolvedValue(detected);
  const { wrapper } = mountCard();
  await flushPromises();
  expect(wrapper.get("[data-testid=cursor-max-detected]").text()).toContain(text);
});

it("treats a failed detection as unknown rather than leaving it loading", async () => {
  plan.mockRejectedValue(new Error("boom"));
  const { wrapper } = mountCard();
  await flushPromises();
  expect(wrapper.get("[data-testid=cursor-max-detected]").text()).toContain("treated as the old plan");
});

it("says it is detecting while the local data is read", () => {
  plan.mockReturnValue(new Promise(() => {}));
  const { wrapper } = mountCard();
  expect(wrapper.get("[data-testid=cursor-max-detected]").text()).toBe("Detected: Detecting…");
});
