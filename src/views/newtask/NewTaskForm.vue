<script setup lang="ts">
// 「新建 Agent 任务」共享表单主体（spec gui-agent-task-launch + gui-agent-console C4）：
// 项目 → 任务来源（直接输入 / 项目待办 + 可选补充）→ Agent → 权限 → 启动。
// 窗口版（NewTaskView）与控制台内嵌版（NewTaskPane）共用；宿主负责主题/语言与启动后的收尾
// （关窗 / 选中新会话），本组件成功后仅 emit `launched`。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  agentTaskReadiness,
  codexLaunchStatus,
  type CodexLaunchStatus,
  cursorLaunchStatus,
  type CursorLaunchStatus,
  newTaskInit,
  newTaskLaunch,
  newTaskProjects,
  newTaskProjectsRefreshed,
  openPath,
  openSettings,
  projectKeyOf,
  todosList,
} from "../../lib/ipc";
import type {
  AgentKind,
  AgentTaskReadiness,
  NewTaskProject,
  PopupSubmitKey,
  TodoEntry,
} from "../../lib/types";
import { AGENT_INSTALL_DOCS } from "../settings/useAgentTasks";
import { primaryModifierPressed, primaryShortcutLabel } from "../../lib/platform";
import { codexLaunchDescription } from "../../lib/codexLaunch";
import { cursorLaunchDescription } from "../../lib/cursorLaunch";
import LaunchPermission from "./LaunchPermission.vue";
import { useTaskSettings } from "./useTaskSettings";

const { t, te } = useI18n();

const props = defineProps<{
  /** 锁定项目（控制台内嵌版）：项目区只读展示、不加载候选下拉。 */
  lockProject?: string | null;
  /** 初始预选（窗口版 URL 参数）；挂载时应用。 */
  initialProject?: string | null;
  initialTodo?: string | null;
}>();

const emit = defineEmits<{ launched: [] }>();

// ===== init =====
const popupSubmitKey = ref<PopupSubmitKey>("cmdEnter");
const submitWithBareEnter = computed(() => popupSubmitKey.value === "enter");
const submitKeyLabel = computed(() =>
  submitWithBareEnter.value ? "↵" : primaryShortcutLabel("enter")
);
/** `agentTasks.permissionPrompt`（G6）：ask 显示单选，另两态为固定模式。 */
const permissionPrompt = ref<string>("ask");
const loaded = ref(false);

const locked = computed(() => !!props.lockProject);

// ===== 项目 =====
const projects = ref<NewTaskProject[]>([]);
const selectedProject = ref("");
/** 所选 workspace 的 git 根 project key（待办按此读取 / 出队）。 */
const projectKey = ref("");

const projectsWorkspace = computed(() =>
  projects.value.filter((p) => p.source === "workspace")
);
const projectsTodos = computed(() =>
  projects.value.filter((p) => p.source !== "workspace")
);

function basename(key: string): string {
  const parts = key.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || key;
}

const lockedLabel = computed(() =>
  props.lockProject ? basename(props.lockProject) : ""
);

/** 预选项目不在候选中（如待办项目未入 workspace 索引）→ 兜底追加，保持选中稳定。 */
function applyProjects(list: NewTaskProject[]): void {
  if (
    selectedProject.value &&
    !list.some((p) => p.path === selectedProject.value)
  ) {
    list = list.concat({
      path: selectedProject.value,
      label: basename(selectedProject.value),
      pinned: false,
      source: "todos",
    });
  }
  projects.value = list;
  if (!selectedProject.value) {
    selectedProject.value = list[0]?.path ?? "";
  }
}

// ===== 任务来源（G3/G7/G8）=====
const todos = ref<TodoEntry[]>([]);
/** 选中的待办**快照**（IM D31 语义：并发删除仍按快照启动）；null = 直接输入。 */
const selectedTodo = ref<{
  id: string;
  text: string;
  auto: boolean;
  project: string;
  attachments: NonNullable<TodoEntry["attachments"]>;
} | null>(null);
/** 选中的待办已被别处删除（仍可按快照启动，仅提示）。 */
const todoMissing = ref(false);
const inputText = ref("");

async function loadTodosFor(path: string): Promise<void> {
  if (!path) {
    projectKey.value = "";
    todos.value = [];
    return;
  }
  try {
    projectKey.value = await projectKeyOf(path);
    todos.value = projectKey.value ? await todosList(projectKey.value) : [];
  } catch (err) {
    console.warn("new-task todos load failed", err);
    todos.value = [];
  }
}

async function onProjectChange(): Promise<void> {
  // 切换项目：清除待办预选（G8），任务来源回到「直接输入」。
  selectedTodo.value = null;
  todoMissing.value = false;
  launchError.value = "";
  await loadTodosFor(selectedProject.value);
}

function selectManual(): void {
  selectedTodo.value = null;
  todoMissing.value = false;
}

function selectTodo(entry: TodoEntry): void {
  selectedTodo.value = {
    id: entry.id,
    text: entry.text,
    auto: entry.auto ?? false,
    project: projectKey.value,
    attachments: (entry.attachments ?? []).map((attachment) => ({ ...attachment })),
  };
  todoMissing.value = false;
}

// ===== Agent（G4/G5）=====
const readiness = ref<AgentTaskReadiness[] | null>(null);
const selectedKind = ref<AgentKind | null>(null);
const desktop = ref<CodexLaunchStatus | null>(null);
/** Where a new Cursor task would run; `null` while unknown, which leaves Cursor judged by its own readiness. */
const cursorLaunch = ref<CursorLaunchStatus | null>(null);
const operationId = ref<string>(crypto.randomUUID());
function agentAvailable(item: AgentTaskReadiness): boolean {
  if (item.kind === "codex") return !!desktop.value?.target;
  if (item.kind === "cursor" && cursorLaunch.value) return !!cursorLaunch.value.target;
  return item.ready;
}
/** Said under Cursor only when its Agents window is involved; the terminal route reads as it always did. */
const cursorRouteNote = computed(() => {
  const status = cursorLaunch.value;
  if (!status || (status.target !== "desktop" && status.preference !== "desktop")) return null;
  return cursorLaunchDescription(status, t);
});
const footerNote = computed(() => {
  if (selectedKind.value === "codex" && desktop.value?.target === "desktop") return "desktop.launchNote";
  if (selectedKind.value === "cursor" && cursorLaunch.value?.target === "desktop") return "newTask.cursorWindowNote";
  return "newTask.launchNote";
});
/** Readiness still lists launch-unsupported families for Settings/doctor, never for this picker. */
const launchableReadiness = computed(
  () => readiness.value?.filter((item) => item.launchSupported) ?? null,
);
const targetReady = computed(() => !!launchableReadiness.value?.find(r => r.kind === selectedKind.value && agentAvailable(r)));

async function loadReadiness(): Promise<void> {
  try {
    const [list, status, cursor] = await Promise.all([
      agentTaskReadiness(),
      codexLaunchStatus(true).catch(() => null),
      cursorLaunchStatus(true).catch(() => null),
    ]);
    desktop.value = status;
    cursorLaunch.value = cursor;
    readiness.value = list;
    // 已选 Agent 不再就绪（重查后）→ 取消选择。
    if (
      selectedKind.value &&
      !list.some((r) => r.launchSupported && r.kind === selectedKind.value && agentAvailable(r))
    ) {
      selectedKind.value = null;
    }
  } catch (err) {
    console.warn("new-task readiness failed", err);
    readiness.value = [];
  }
}

function selectAgent(item: AgentTaskReadiness): void {
  if (!item.launchSupported || !agentAvailable(item)) return;
  selectedKind.value = item.kind;
}

/** Missing readiness links: binary opens vendor docs; integration capabilities open the Agent card. */
async function openIssue(
  kind: AgentKind,
  issue: "binary" | "lifecycle" | "integration"
): Promise<void> {
  try {
    if (issue === "binary") {
      await openPath(AGENT_INSTALL_DOCS[kind]);
    } else if (issue === "lifecycle") {
      await openSettings(`integration#lifecycle-${kind}`);
    } else {
      await openSettings(`integration#integration-${kind}`);
    }
  } catch (err) {
    console.warn("new-task open issue failed", err);
  }
}

// ===== 权限（G6）=====
/** ask 模式下的用户选择（不预选，IM D19 同语义）；固定模式下忽略。 */
const permissionChoice = ref<"agent-default" | "yolo" | null>(null);
const effectivePermission = computed<"agent-default" | "yolo" | null>(() => {
  if (selectedKind.value === "pi") return "agent-default";
  if (permissionPrompt.value === "agent-default") return "agent-default";
  if (permissionPrompt.value === "yolo") return "yolo";
  return permissionChoice.value;
});

// Model preferences and per-task Fast / Max settings follow the iOS composer rules.
const {
  options: taskOptions, loading: optionsLoading, failed: optionsFailed,
  modelId, effortId, model: pickedModel, effortChoices, defaultEffort, offersModels,
  speedControl, supportsFast, fast, fastWasReset, speed, maxControl,
  maxRequired, maxBlocked, allowMax, reload: loadTaskOptions,
  selectModel, selectEffort, setFast, remember: rememberChoices, resetTask: resetTaskChoices,
} = useTaskSettings(selectedKind);
const modelSearch = ref("");
watch(selectedKind, () => { modelSearch.value = ""; });
const filteredModels = computed(() => {
  const all = taskOptions.value?.models ?? [];
  const needle = modelSearch.value.trim().toLowerCase();
  const matches = all.filter(m => !needle || m.label.toLowerCase().includes(needle) || m.id.toLowerCase().includes(needle));
  // Keep the current selection visible even while the user searches for its replacement.
  if (pickedModel.value && !matches.some(m => m.id === pickedModel.value?.id)) return [pickedModel.value, ...matches];
  return matches;
});
function chooseModel(event: Event): void {
  selectModel((event.target as HTMLSelectElement).value || null);
  modelSearch.value = "";
}
const willUse = computed(() => {
  if (!maxControl.value) return null;
  return { label: pickedModel.value?.label ?? taskOptions.value?.default.label ?? t("newTask.cursorDefaultModel"), maxRequired: maxRequired.value };
});
function maxMark(required: boolean | null | undefined): string {
  if (required === true) return t("newTask.cursorMaxRequiredMark");
  if (required === false) return t("newTask.cursorMaxNotRequiredMark");
  return t("newTask.cursorMaxUnknownMark");
}
const maxUnknown = computed(() => maxControl.value && !allowMax.value && maxRequired.value == null);
function effortLabel(id: string): string {
  const key = `newTask.efforts.${id}`;
  return te(key) ? t(key) : id;
}

// ===== 任务拼装与校验（spec §3.3）=====
const MAX_TASK_CHARS = 3000;

/** 最终任务 = 待办原文 + 空行 + 补充（补充为空则仅原文）；直接输入 = 输入本身。 */
const finalTask = computed(() => {
  const input = inputText.value.trim();
  if (selectedTodo.value) {
    const base = selectedTodo.value.text.trim();
    return input ? `${base}\n\n${input}` : base;
  }
  return input;
});
const taskChars = computed(() => [...finalTask.value].length);
const tooLong = computed(() => taskChars.value > MAX_TASK_CHARS);

// A pre-upgrade pending create must keep its original receipt and inherited speed. Assigning
// a new id merely because this form now defaults to explicit standard could duplicate the task.
const pendingVersion = ref(0);
function readPendingCreate(): { storageKey: string; key: string; submittedSpeed: "standard" | "fast" | undefined; inheritedSpeed: boolean; id: string | null } | null {
  const kind = selectedKind.value;
  if (kind !== "codex" && kind !== "cursor") return null;
  const base = [selectedProject.value, finalTask.value, effectivePermission.value, selectedTodo.value?.project, selectedTodo.value?.id, selectedTodo.value?.attachments];
  const storageKey = `${kind}-desktop-pending-create`;
  let saved: { key: string; id: string } | null = null;
  try { saved = JSON.parse(localStorage.getItem(storageKey) || "null"); } catch { /* Ignore malformed local state. */ }
  const oldKeys = kind === "codex"
    ? [JSON.stringify(base), JSON.stringify([selectedProject.value, finalTask.value, effectivePermission.value, selectedTodo.value?.id])]
    : [JSON.stringify([...base, modelId.value ?? "", maxControl.value ? allowMax.value : false])];
  const legacy = !!saved && typeof saved.id === "string" && !effortId.value && !fast.value
    && (kind === "cursor" || !modelId.value) && oldKeys.includes(saved.key);
  const submittedSpeed = legacy ? undefined : speed.value;
  const key = legacy ? saved!.key : JSON.stringify([...base, modelId.value, effortId.value, submittedSpeed, maxControl.value ? allowMax.value : false]);
  return { storageKey, key, submittedSpeed, inheritedSpeed: legacy && kind === "codex", id: saved && typeof saved.id === "string" && saved.key === key ? saved.id : null };
}
const pendingCreate = computed(() => { pendingVersion.value; return readPendingCreate(); });

const canLaunch = computed(
  () =>
    !launching.value && !optionsLoading.value && !optionsFailed.value && targetReady.value &&
    !!selectedProject.value &&
    !!selectedKind.value &&
    !!effectivePermission.value &&
    finalTask.value.length > 0 &&
    !tooLong.value &&
    !maxBlocked.value
);

// ===== 启动 =====
const launching = ref(false);
const launchError = ref("");

async function launch(): Promise<void> {
  if (!canLaunch.value || !selectedKind.value || !effectivePermission.value) {
    return;
  }
  launching.value = true;
  launchError.value = "";
  try {
    rememberChoices();
    const pending = readPendingCreate();
    if (pending) {
      operationId.value = pending.id ?? crypto.randomUUID();
      localStorage.setItem(pending.storageKey, JSON.stringify({ key: pending.key, id: operationId.value }));
      pendingVersion.value++;
    }
    await newTaskLaunch({
      operationId: operationId.value,
      workspace: selectedProject.value,
      kind: selectedKind.value,
      permission: effectivePermission.value,
      task: finalTask.value,
      todoProject: selectedTodo.value?.project ?? null,
      todoId: selectedTodo.value?.id ?? null,
      todoAttachments:
        selectedTodo.value?.attachments.map((attachment) => ({
          id: attachment.id,
          name: attachment.name,
          path: attachment.path,
          sourcePath: attachment.sourcePath,
          storage: attachment.storage,
        })) ?? [],
      model: pickedModel.value?.id ?? null,
      effort: effortId.value,
      speed: pending ? pending.submittedSpeed : speed.value,
      allowMax: maxControl.value ? allowMax.value : undefined,
    });
    if (pending) localStorage.removeItem(pending.storageKey);
    pendingVersion.value++;
    operationId.value=crypto.randomUUID();
    resetTaskChoices();
  } catch (err) {
    const reason = String(err).replace(/^Error: /, "");
    const key = `newTask.optionErrors.${reason}`;
    launchError.value = t("newTask.launchFailed", { e: te(key) ? t(key) : String(err) });
    return;
  } finally {
    launching.value = false;
  }
  // 成功收尾由宿主决定（窗口版关窗 / 控制台版选中新会话）。
  emit("launched");
}

function onInputKeydown(e: KeyboardEvent): void {
  // 与弹窗/待办窗口一致的提交快捷键；IME 组词中的 Enter 不当提交。
  if (e.key !== "Enter") return;
  if (e.isComposing || (e as KeyboardEvent & { keyCode?: number }).keyCode === 229) {
    return;
  }
  const mod = primaryModifierPressed(e);
  const anyMod = e.metaKey || e.ctrlKey || e.shiftKey || e.altKey;
  const isPrimarySendMod = mod && !e.shiftKey && !e.altKey;
  const shouldSubmit = submitWithBareEnter.value ? !anyMod : isPrimarySendMod;
  if (!shouldSubmit) return;
  e.preventDefault();
  void launch();
}

// ===== 预选与重置（窗口版 goto 事件经 defineExpose 调用）=====
async function applyPreselect(
  project: string | null,
  todoId: string | null
): Promise<void> {
  if (project) {
    selectedProject.value = project;
    applyProjects(projects.value.slice());
  }
  selectedTodo.value = null;
  todoMissing.value = false;
  launchError.value = "";
  inputText.value = "";
  resetTaskChoices();
  await loadTodosFor(selectedProject.value);
  if (todoId) {
    const entry = todos.value.find((e) => e.id === todoId);
    if (entry) selectTodo(entry);
  }
}

defineExpose({ applyPreselect });

function applyPopupSubmitKey(value: unknown): void {
  if (value === "enter" || value === "cmdEnter") {
    popupSubmitKey.value = value;
  }
}

let unlistenSettings: UnlistenFn | null = null;
let unlistenTodos: UnlistenFn | null = null;

function onWindowFocus(): void {
  void loadReadiness();
  void loadTaskOptions();
}

onMounted(async () => {
  window.addEventListener("focus", onWindowFocus);
  try {
    const init = await newTaskInit();
    applyPopupSubmitKey(init.popupSubmitKey);
    permissionPrompt.value = init.permissionPrompt;
  } catch {
    /* 读取失败：保持兜底默认 */
  }
  if (props.lockProject) {
    selectedProject.value = props.lockProject;
  } else if (props.initialProject) {
    selectedProject.value = props.initialProject;
  }

  unlistenSettings = await listen<{ popupSubmitKey?: PopupSubmitKey }>(
    "settings-updated",
    (e) => {
      applyPopupSubmitKey(e.payload.popupSubmitKey);
      void loadReadiness();
      void loadTaskOptions();
    }
  );
  // todos.json 被任意进程改写 → 重载所选项目待办；选中待办被删时保留快照并提示（IM D31）。
  unlistenTodos = await listen("todos-updated", async () => {
    await loadTodosFor(selectedProject.value);
    if (
      selectedTodo.value &&
      !todos.value.some((e) => e.id === selectedTodo.value?.id)
    ) {
      todoMissing.value = true;
    }
  });

  // 首屏：本地项目列表 + 所选项目待办（瞬时）；Agent 就绪与冷扫描合并放后台。
  // 锁定项目（控制台内嵌）不加载候选下拉。
  try {
    if (!locked.value) {
      applyProjects(await newTaskProjects());
    }
    await applyPreselect(null, props.initialTodo ?? null);
  } catch (err) {
    console.warn("new-task init load failed", err);
  } finally {
    loaded.value = true;
  }
  void loadReadiness();
  if (!locked.value) {
    void (async () => {
      try {
        applyProjects(await newTaskProjectsRefreshed());
      } catch (err) {
        console.warn("new-task projects refresh failed", err);
      }
    })();
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("focus", onWindowFocus);
  unlistenSettings?.();
  unlistenTodos?.();
});
</script>

<template>
  <div class="nt-form">
    <div class="nt-body">
      <div v-if="!loaded" class="nt-empty">
        <span class="nt-spinner" />
      </div>

      <div v-else-if="!locked && !projects.length" class="nt-empty">
        <p class="nt-empty-title">{{ t("newTask.noProjects") }}</p>
        <p class="nt-empty-hint">{{ t("newTask.noProjectsHint") }}</p>
      </div>

      <template v-else>
        <!-- 项目 -->
        <section class="nt-section">
          <label class="nt-label" for="nt-project">{{ t("newTask.projectLabel") }}</label>
          <p v-if="locked" class="nt-locked-project" :title="lockProject ?? ''">
            {{ lockedLabel }}
            <span class="nt-locked-path">{{ lockProject }}</span>
          </p>
          <select
            v-else
            id="nt-project"
            v-model="selectedProject"
            class="nt-select"
            @change="onProjectChange"
          >
            <optgroup
              v-if="projectsWorkspace.length"
              :label="t('newTask.projectSectionWorkspace')"
            >
              <option
                v-for="p in projectsWorkspace"
                :key="p.path"
                :value="p.path"
                :title="p.path"
              >
                {{ p.pinned ? "★ " : "" }}{{ p.label }}
              </option>
            </optgroup>
            <optgroup
              v-if="projectsTodos.length"
              :label="t('newTask.projectSectionTodos')"
            >
              <option
                v-for="p in projectsTodos"
                :key="p.path"
                :value="p.path"
                :title="p.path"
              >
                {{ p.label }}
              </option>
            </optgroup>
          </select>
        </section>

        <!-- 任务来源：直接输入 / 项目待办（全部列出、可滚动，G9） -->
        <section class="nt-section">
          <span class="nt-label">{{ t("newTask.taskLabel") }}</span>
          <div v-if="todos.length || selectedTodo" class="nt-choices">
            <button
              type="button"
              class="nt-choice"
              :class="{ active: !selectedTodo }"
              @click="selectManual"
            >
              <span class="nt-radio" :class="{ on: !selectedTodo }" />
              <span class="nt-choice-text">{{ t("newTask.manualEntry") }}</span>
            </button>
            <button
              v-for="e in todos"
              :key="e.id"
              type="button"
              class="nt-choice"
              :class="{ active: selectedTodo?.id === e.id }"
              @click="selectTodo(e)"
            >
              <span class="nt-radio" :class="{ on: selectedTodo?.id === e.id }" />
              <span
                class="nt-choice-text"
                :class="{ clamp: selectedTodo?.id !== e.id }"
              >
                <!-- 待办前缀：与飞书/钉钉任务卡的琥珀色【TODO】标记同语义（IM D29）。 -->
                <span class="nt-todo-tag">{{ t("newTask.todoTag") }}</span
                ><span v-if="e.auto" class="nt-auto">⚡</span>{{ e.text }}
                <span v-if="e.attachments?.length" class="nt-file-count">
                  {{ t("common.attachmentBadge", { n: e.attachments.length }) }}
                </span>
              </span>
            </button>
            <!-- 快照兜底：选中的待办已被删除（不再出现在列表）仍显示为选中项。 -->
            <button
              v-if="selectedTodo && !todos.some((e) => e.id === selectedTodo?.id)"
              type="button"
              class="nt-choice active"
            >
              <span class="nt-radio on" />
              <span class="nt-choice-text">
                <span class="nt-todo-tag">{{ t("newTask.todoTag") }}</span
                ><span v-if="selectedTodo.auto" class="nt-auto">⚡</span
                >{{ selectedTodo.text }}
                <span
                  v-if="selectedTodo.attachments?.length"
                  class="nt-file-count"
                >
                  {{ t("common.attachmentBadge", { n: selectedTodo.attachments.length }) }}
                </span>
              </span>
            </button>
          </div>
          <div
            v-if="selectedTodo?.attachments.length"
            class="nt-selected-files"
          >
            <span
              v-for="attachment in selectedTodo.attachments"
              :key="attachment.id"
              class="nt-selected-file"
              :title="attachment.sourcePath"
            >
              📎 {{ attachment.name }}
            </span>
          </div>
          <p v-if="todoMissing" class="nt-note">{{ t("newTask.todoMissing") }}</p>
          <label class="nt-sublabel" for="nt-input">
            {{ selectedTodo ? t("newTask.supplementLabel") : todos.length ? t("newTask.taskLabel") : "" }}
          </label>
          <textarea
            id="nt-input"
            v-model="inputText"
            class="nt-input"
            rows="3"
            :placeholder="
              selectedTodo
                ? t('newTask.supplementPlaceholder')
                : t('newTask.taskPlaceholder')
            "
            @keydown="onInputKeydown"
          />
          <p v-if="tooLong" class="nt-error">
            {{ t("newTask.tooLong", { n: taskChars }) }}
          </p>
        </section>

        <!-- Agent：仅列出可启动的 family（readiness 仍为设置/doctor 提供全量） -->
        <section class="nt-section">
          <span class="nt-label">{{ t("newTask.agentLabel") }}</span>
          <div v-if="readiness === null" class="nt-agent-loading">
            <span class="nt-spinner small" />
            <span>{{ t("newTask.agentLoading") }}</span>
          </div>
          <div v-else class="nt-agents">
            <button
              v-for="item in (launchableReadiness ?? [])"
              :key="item.kind"
              type="button"
              class="nt-agent"
              :class="{
                active: selectedKind === item.kind,
                disabled: !agentAvailable(item),
              }"
              @click="selectAgent(item)"
            >
              <span class="nt-radio" :class="{ on: selectedKind === item.kind }" />
              <span class="nt-agent-main">
                <span class="nt-agent-name">{{ item.label }}</span>
                <span v-if="item.kind === 'codex' && desktop" class="nt-agent-sub">
                  {{ codexLaunchDescription(desktop, t) }}
                </span>
                <span v-else-if="item.kind === 'cursor' && cursorRouteNote" class="nt-agent-sub">
                  {{ cursorRouteNote }}
                </span>
                <span v-else-if="item.ready" class="nt-agent-sub" :title="item.executable ?? ''">
                  {{ item.integrationMode.toUpperCase() }} · {{ item.executable }}
                </span>
                <span v-else class="nt-agent-issues">
                  <template
                    v-for="issue in (['binary', 'lifecycle', 'integration'] as const)"
                    :key="issue"
                  >
                    <span
                      v-if="
                        issue === 'binary'
                          ? item.binaryReady
                          : issue === 'lifecycle'
                            ? item.lifecycleReady
                            : item.integrationReady
                      "
                      class="nt-issue ok"
                    >
                      {{ t(`newTask.ready${issue.charAt(0).toUpperCase() + issue.slice(1)}`) }} ✓
                    </span>
                    <a
                      v-else
                      class="nt-issue bad"
                      href="#"
                      @click.prevent.stop="openIssue(item.kind, issue)"
                    >
                      {{ t(`newTask.ready${issue.charAt(0).toUpperCase() + issue.slice(1)}`) }} ✗
                    </a>
                  </template>
                </span>
              </span>
            </button>
          </div>
        </section>

        <section v-if="selectedKind" class="nt-section" data-testid="nt-task-settings" :inert="launching">
          <p v-if="optionsLoading" class="nt-note">{{ t("newTask.optionsLoading") }}</p>
          <template v-else-if="optionsFailed">
            <p class="nt-error">{{ t("newTask.optionsFailed") }}</p>
            <button type="button" class="nt-btn" @click="loadTaskOptions">{{ t("newTask.retryOptions") }}</button>
          </template>
          <template v-else>
            <div class="nt-options-header">
              <label class="nt-label" for="nt-task-model">{{ t("newTask.modelLabel") }}</label>
              <button type="button" class="nt-refresh-options" @click="loadTaskOptions">{{ t("newTask.refreshOptions") }}</button>
            </div>
            <input
              v-if="(taskOptions?.models.length ?? 0) > 12"
              v-model="modelSearch" type="search" class="nt-select" data-testid="nt-model-search"
              :placeholder="t('newTask.searchModels')" :aria-label="t('newTask.searchModels')"
            />
            <select id="nt-task-model" :value="modelId ?? ''" class="nt-select" :disabled="!offersModels" @change="chooseModel">
              <option value="">{{ t("newTask.modelDefault") }}</option>
              <option v-for="m in filteredModels" :key="m.id" :value="m.id" :title="m.note ?? ''">
                {{ m.label }}{{ maxControl && m.maxRequired != null ? ` · ${maxMark(m.maxRequired)}` : "" }}
              </option>
            </select>
            <p v-if="!offersModels" class="nt-note">{{ t("newTask.modelsUnavailable") }}</p>
            <p v-else-if="!modelId" class="nt-note">{{ t("newTask.defaultMeans", { model: taskOptions?.default.label ?? t('newTask.agentOwnSetting') }) }}</p>
            <p v-else-if="pickedModel?.note" class="nt-note">{{ pickedModel.note }}</p>
            <template v-if="offersModels && effortChoices.length">
              <label class="nt-label" for="nt-task-effort">{{ t("newTask.effortLabel") }}</label>
              <select id="nt-task-effort" :value="effortId ?? ''" class="nt-select" @change="selectEffort(($event.target as HTMLSelectElement).value || null)">
                <option value="">{{ defaultEffort ? t('newTask.effortDefault', { effort: effortLabel(defaultEffort) }) : t('newTask.modelDefault') }}</option>
                <option v-for="level in effortChoices" :key="level" :value="level">{{ effortLabel(level) }}</option>
              </select>
            </template>
            <p v-if="pendingCreate?.inheritedSpeed" class="nt-note" data-testid="nt-legacy-retry">{{ t("newTask.legacyRetry") }}</p>
            <template v-if="selectedKind === 'codex'">
              <template v-if="speedControl">
                <button type="button" class="nt-choice nt-fast" :class="{ active: fast }" role="switch" :aria-checked="fast" :disabled="!supportsFast" @click="setFast(!fast)">
                  <span class="nt-check" :class="{ on: fast }" />
                  <span class="nt-choice-text">{{ t("newTask.fastMode") }}<span class="nt-choice-sub">{{ t("newTask.fastDesc") }}</span></span>
                </button>
                <p v-if="!supportsFast" class="nt-note">{{ t(modelId ? 'newTask.fastUnsupported' : 'newTask.fastChooseModel') }}</p>
                <p v-if="fastWasReset" class="nt-note" data-testid="nt-fast-reset">{{ t("newTask.fastReset") }}</p>
              </template>
              <p v-else class="nt-note">{{ t("newTask.fastUnavailable") }}</p>
            </template>
          </template>
        </section>

        <section v-if="maxControl && !optionsLoading" class="nt-section" data-testid="nt-cursor-max" :inert="launching">
          <p v-if="willUse" class="nt-note nt-will-use" :class="{ bad: maxBlocked }">
            {{ t("newTask.cursorWillUse", { model: willUse.label, max: maxMark(willUse.maxRequired) }) }}
          </p>
          <button type="button" class="nt-choice nt-allow-max" :class="{ active: allowMax, danger: allowMax }" role="switch" :aria-checked="allowMax" @click="allowMax = !allowMax">
            <span class="nt-check" :class="{ on: allowMax }" />
            <span class="nt-choice-text">{{ t("newTask.allowMax") }}<span class="nt-choice-sub">{{ t("newTask.allowMaxDesc") }}</span></span>
          </button>
          <p v-if="maxBlocked" class="nt-error" data-testid="nt-max-blocked">{{ t("newTask.maxBlocked", { model: willUse?.label ?? '' }) }}</p>
          <p v-else-if="maxUnknown" class="nt-note" data-testid="nt-max-unknown">{{ t("newTask.maxUnknownNote") }}</p>
        </section>

        <LaunchPermission
          v-if="selectedKind !== 'pi'"
          v-model="permissionChoice"
          :permission-prompt="permissionPrompt"
        />
        <p v-else class="nt-sublabel">{{ t("newTask.piPermissionHint") }}</p>
      </template>
    </div>

    <footer v-if="loaded && (locked || projects.length)" class="nt-footer">
      <p v-if="launchError" class="nt-error">{{ launchError }}</p>
      <div class="nt-footer-row">
        <span class="nt-note">{{ t(footerNote) }}</span>
        <button
          type="button"
          class="nt-btn nt-btn-launch"
          :disabled="!canLaunch"
          @click="launch"
        >
          {{ launching ? t("newTask.launching") : t("newTask.launch") }}
          <kbd v-if="!launching" class="sc">{{ submitKeyLabel }}</kbd>
        </button>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.nt-options-header { display: flex; align-items: center; justify-content: space-between; }
.nt-refresh-options { color: var(--text-secondary); background: none; border: 0; font: inherit; font-size: 12px; cursor: pointer; }

.nt-form {
  display: flex;
  flex-direction: column;
  flex: 1 1 auto;
  min-height: 0;
  color: var(--text-primary);
}
.nt-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.nt-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 100%;
  text-align: center;
}
.nt-empty-title {
  font-size: 14px;
  font-weight: 600;
  margin: 0;
}
.nt-empty-hint {
  font-size: 12px;
  color: var(--text-secondary);
  margin: 0;
  max-width: 320px;
}
.nt-spinner {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: 2px solid color-mix(in srgb, var(--text-primary) 18%, transparent);
  border-top-color: var(--text-secondary);
  animation: nt-spin 0.7s linear infinite;
}
.nt-spinner.small {
  width: 14px;
  height: 14px;
}
@keyframes nt-spin {
  to {
    transform: rotate(360deg);
  }
}
.nt-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.nt-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}
.nt-sublabel {
  font-size: 11px;
  color: var(--text-secondary);
}
.nt-sublabel:empty {
  display: none;
}
.nt-locked-project {
  margin: 0;
  font-size: 12.5px;
  font-weight: 600;
}
.nt-locked-path {
  display: block;
  font-size: 11px;
  font-weight: 400;
  color: var(--text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.nt-select {
  width: 100%;
  min-width: 0;
  appearance: auto;
  border: var(--hairline) solid var(--border);
  border-radius: 7px;
  background: var(--control-bg);
  color: var(--text-primary);
  font-size: 12px;
  padding: 5px 8px;
  box-shadow: var(--clickable-shadow);
}
/* 任务来源 / 权限：radio 卡片列表（待办全部列出、区域内滚动，G9）。 */
.nt-choices {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 180px;
  overflow-y: auto;
}
.nt-choice {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  text-align: left;
  padding: 7px 10px;
  border: var(--hairline) solid var(--border);
  border-radius: 8px;
  background: var(--bg-elevated);
  color: var(--text-primary);
  font: inherit;
  font-size: 12.5px;
  line-height: 1.4;
  cursor: pointer;
}
.nt-choice:hover {
  background: var(--control-hover-bg);
}
.nt-choice.active {
  border-color: color-mix(in srgb, #0a84ff 55%, var(--border));
  background: color-mix(in srgb, #0a84ff 8%, var(--bg-elevated));
}
.nt-choice.danger.active {
  border-color: color-mix(in srgb, #ff453a 55%, var(--border));
  background: color-mix(in srgb, #ff453a 8%, var(--bg-elevated));
}
.nt-radio {
  flex: 0 0 auto;
  width: 12px;
  height: 12px;
  margin-top: 2px;
  border-radius: 50%;
  border: 1.4px solid var(--text-secondary);
  box-sizing: border-box;
}
.nt-radio.on {
  border-color: #0a84ff;
  background: radial-gradient(circle, #0a84ff 0 3.5px, transparent 4px);
}
/* Switch-like control (Allow Max): a checkbox, since it is on/off rather than one of several. */
.nt-check {
  flex: 0 0 auto;
  width: 12px;
  height: 12px;
  margin-top: 2px;
  border-radius: 3px;
  border: 1.4px solid var(--text-secondary);
  box-sizing: border-box;
}
.nt-check.on {
  border-color: #ff453a;
  background: #ff453a;
}
.nt-will-use.bad {
  color: #ff453a;
}
.nt-choice.danger .nt-radio.on {
  border-color: #ff453a;
  background: radial-gradient(circle, #ff453a 0 3.5px, transparent 4px);
}
.nt-choice-text {
  min-width: 0;
  white-space: pre-wrap;
  word-break: break-word;
}
.nt-choice-text.clamp {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.nt-choice-sub {
  display: block;
  font-size: 11px;
  color: var(--text-secondary);
  margin-top: 1px;
}
.nt-auto {
  color: #ff9f0a;
  margin-right: 3px;
}
/* Todo prefix tag: same semantics as the amber 【TODO】 marker on Feishu/DingTalk task cards (IM D29). */
.nt-todo-tag {
  color: #ff9f0a;
  font-weight: 600;
  margin-right: 2px;
}
.nt-file-count {
  margin-left: 6px;
  color: var(--text-secondary);
  font-size: inherit;
  white-space: nowrap;
}
.nt-selected-files {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  margin-top: 7px;
}
.nt-selected-file {
  max-width: 220px;
  overflow: hidden;
  padding: 3px 7px;
  border: var(--hairline) solid var(--border);
  border-radius: 6px;
  color: var(--text-secondary);
  font-size: 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.nt-input {
  display: block;
  width: 100%;
  min-width: 0;
  min-height: 60px;
  max-height: 150px;
  resize: vertical;
  overflow-y: auto;
  border: var(--hairline) solid var(--control-border);
  border-radius: 7px;
  background: var(--control-bg);
  color: var(--text-primary);
  font: inherit;
  font-size: 12px;
  line-height: 1.45;
  padding: 7px 9px;
  box-shadow: var(--clickable-shadow);
  box-sizing: border-box;
}
.nt-input:focus,
.nt-input:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring), var(--clickable-shadow);
}
/* Agent 区（G5）：五家全列；未就绪灰显 + 原因链接。 */
.nt-agent-loading {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-secondary);
  padding: 6px 2px;
}
.nt-agents {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.nt-agent {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  text-align: left;
  padding: 7px 10px;
  border: var(--hairline) solid var(--border);
  border-radius: 8px;
  background: var(--bg-elevated);
  color: var(--text-primary);
  font: inherit;
  cursor: pointer;
}
.nt-agent:hover:not(.disabled) {
  background: var(--control-hover-bg);
}
.nt-agent.active {
  border-color: color-mix(in srgb, #0a84ff 55%, var(--border));
  background: color-mix(in srgb, #0a84ff 8%, var(--bg-elevated));
}
.nt-agent.disabled {
  cursor: default;
}
.nt-agent.disabled .nt-agent-name,
.nt-agent.disabled .nt-radio {
  opacity: 0.45;
}
.nt-agent-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.nt-agent-name {
  font-size: 12.5px;
  font-weight: 600;
}
.nt-agent-sub {
  font-size: 11px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.nt-agent-issues {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  font-size: 11px;
}
.nt-issue.ok {
  color: var(--text-secondary);
  opacity: 0.6;
}
.nt-issue.bad {
  color: #ff453a;
  text-decoration: none;
}
.nt-issue.bad:hover {
  text-decoration: underline;
}
.nt-badge-danger {
  display: inline-block;
  margin-left: 6px;
  padding: 0 5px;
  border-radius: 5px;
  background: color-mix(in srgb, #ff453a 16%, transparent);
  color: #ff453a;
  font-size: 10px;
  font-weight: 700;
  line-height: 1.6;
  vertical-align: 1px;
}
.nt-permission-fixed {
  margin: 0;
  font-size: 12.5px;
}
.nt-note {
  margin: 0;
  font-size: 11px;
  color: var(--text-secondary);
}
.nt-error {
  margin: 0;
  font-size: 11.5px;
  color: #ff453a;
  white-space: pre-wrap;
  word-break: break-word;
}
.nt-footer {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 14px 12px;
  border-top: var(--hairline) solid var(--border);
}
.nt-footer-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.nt-btn {
  appearance: none;
  flex: 0 0 auto;
  border: var(--hairline) solid var(--control-border);
  background: var(--control-bg);
  color: var(--text-primary);
  font-size: 12px;
  font-weight: 600;
  padding: 5px 12px;
  border-radius: 7px;
  cursor: pointer;
  box-shadow: var(--clickable-shadow);
}
.nt-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.nt-btn-launch {
  border-color: transparent;
  background: #0a84ff;
  color: #fff;
}
.nt-btn-launch:hover:not(:disabled) {
  background: #0071e3;
}
.nt-btn .sc {
  margin-left: 6px;
  font-size: 11px;
  line-height: 1;
  opacity: 0.85;
  font-family: inherit;
  border: none;
  background: transparent;
  padding: 0;
  color: inherit;
}
</style>
