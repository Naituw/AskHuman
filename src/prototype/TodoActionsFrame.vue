<script setup lang="ts">
// One popup window for the "complete / delete / add on every todo list" prototype (docs/specs/todo-list-actions.md).
// The real PopupView runs inside a fake window and lists the mock project's todos itself (the folded section of an
// ordinary ask, the todo options of What's next and of the Stop card). What this prototype adds is injected into that DOM
// so it sits among the product's own rows: the hover actions (✓ complete, ✕ delete), the "completing" and
// "confirm delete" states and the add row at the end of the list. State lives in `data-ta-*` attributes, not classes:
// Vue rewrites a row's `class` whenever its selection changes, attributes it does not own survive.
// The scene comes from the URL (see `Scene` in todoActionsMock.ts).
import { onBeforeUnmount, onMounted } from "vue";
import PopupView from "../views/PopupView.vue";
import { readParams, type Scene } from "./todoActionsMock";
import "./replyMenu.css";
import "./todoActions.css";

const params = readParams();
const windowStyle = { "--rp-win-w": `${params.width}px`, "--rp-win-h": `${params.height}px` };

type RowState = "hover" | "completing" | "confirm" | "added" | "";
interface SceneSpec {
  /** Todo rows (0-based) that are picked, as if the user had tapped them. */
  picks: number[];
  /** Per todo row: the state to draw. */
  states: Record<number, RowState>;
  /** Which of a hovered row's buttons the pointer is on. */
  hot?: "done" | "del";
  add: "ghost" | "editing";
  typed?: string;
}

const SPECS: Record<Scene, SceneSpec> = {
  ask: { picks: [1], states: { 2: "hover" }, hot: "done", add: "ghost" },
  "ask-states": { picks: [1], states: { 0: "completing", 3: "confirm" }, add: "ghost" },
  "ask-add": { picks: [1], states: { 5: "added" }, add: "editing", typed: "给 CLI 增加 --escalate 参数的文档" },
  next: { picks: [1], states: { 2: "hover" }, hot: "done", add: "ghost" },
  "next-states": { picks: [1], states: { 0: "completing", 3: "confirm" }, add: "ghost" },
  "next-add": { picks: [1], states: { 5: "added" }, add: "editing", typed: "给 CLI 增加 --escalate 参数的文档" },
  "ask-empty": { picks: [], states: {}, add: "ghost" },
  "next-empty": { picks: [], states: {}, add: "ghost" },
  "stop-now": { picks: [], states: {}, add: "ghost" },
  stop: { picks: [1], states: { 1: "hover" }, hot: "done", add: "ghost" },
};
const spec = SPECS[params.scene];
const hoverRow = params.hover >= 0 ? params.hover : undefined;

const ICON_DONE =
  '<svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><path d="M2.4 6.5 4.9 9 9.7 3.3" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>';
const ICON_DEL =
  '<svg viewBox="0 0 12 12" width="9" height="9" aria-hidden="true"><line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>';
const ICON_PLUS =
  '<svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><path d="M6 2v8M2 6h8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>';

function el(html: string): HTMLElement {
  const tpl = document.createElement("template");
  tpl.innerHTML = html.trim();
  return tpl.content.firstElementChild as HTMLElement;
}

function actions(state: RowState): HTMLElement {
  if (state === "completing") return el(`<span class="ta-acts ta-acts-on"><button class="ta-undo" type="button">撤销</button></span>`);
  if (state === "confirm") return el(`<span class="ta-acts ta-acts-on"><button class="ta-confirm" type="button">确认删除</button></span>`);
  const hot = state === "hover" ? spec.hot : undefined;
  const tip = hot === "done" ? `<span class="ta-tip">标记完成</span>` : hot === "del" ? `<span class="ta-tip">删除</span>` : "";
  return el(
    `<span class="ta-acts">` +
      `<button class="ta-btn ta-done" data-hot="${hot === "done"}" type="button" aria-label="标记完成">${ICON_DONE}${hot === "done" ? tip : ""}</button>` +
      `<button class="ta-btn ta-del" data-hot="${hot === "del"}" type="button" aria-label="删除">${ICON_DEL}${hot === "del" ? tip : ""}</button>` +
      `</span>`,
  );
}

function decorateRow(row: HTMLElement, index: number, stand: "ask" | "next"): void {
  if (row.dataset.ta) return;
  const state: RowState = (index === hoverRow ? "hover" : spec.states[index]) ?? "";
  row.dataset.ta = "1";
  row.dataset.taState = state;
  row.querySelectorAll(".todo-del, .todo-del-confirm").forEach((node) => node.remove());
  const acts = actions(state);
  const hotkey = row.querySelector(".opt-sc");
  if (hotkey) row.insertBefore(acts, hotkey);
  else row.appendChild(acts);
  if (state === "completing") {
    const check = row.querySelector<HTMLElement>(".check");
    if (check) check.textContent = "✓";
  }
  if (stand === "ask" && state === "completing") row.classList.remove("selected");
}

function addRow(): HTMLElement {
  if (spec.add === "editing") {
    return el(
      `<div class="ta-add editing" data-ta-add="1">` +
        `<span class="ta-add-ico">${ICON_PLUS}</span>` +
        `<span class="ta-add-field"><span class="ta-typed">${spec.typed ?? ""}</span><i class="ta-caret"></i></span>` +
        `<span class="ta-add-hint"><kbd>⌘↵</kbd> 添加</span>` +
        `</div>`,
    );
  }
  return el(`<div class="ta-add" data-ta-add="1"><span class="ta-add-ico">${ICON_PLUS}</span><span class="ta-add-label">添加待办</span></div>`);
}

function pick(rows: HTMLElement[]): void {
  spec.picks.forEach((i) => {
    const row = rows[i];
    if (row && !row.dataset.taPicked && (spec.states[i] ?? "") !== "completing") {
      row.dataset.taPicked = "1";
      row.click();
    }
  });
}

/** A project without todos still gets a way in: the folded header reads "＋ 添加待办". */
function decorateEmpty(root: HTMLElement): void {
  const content = root.querySelector<HTMLElement>(".popup-main > .content");
  if (!content || !content.querySelector(".question-card, .q-card, .options, .qpane, textarea")) return;
  if (content.querySelector(".ta-empty")) return;
  content.appendChild(
    el(
      `<div class="todo-section ta-empty"><button class="todo-toggle" type="button">` +
        `<span class="ta-pill-ico">${ICON_PLUS}</span><span class="todo-title">添加待办</span></button></div>`,
    ),
  );
}

function decorateAsk(root: HTMLElement): void {
  const toggle = root.querySelector<HTMLButtonElement>(".todo-toggle");
  if (toggle?.getAttribute("aria-expanded") === "false") {
    toggle.click();
    return;
  }
  const list = root.querySelector<HTMLElement>(".todo-options");
  if (!list) return;
  const rows = [...list.querySelectorAll<HTMLElement>(":scope > .option.todo-option")];
  rows.forEach((row, i) => decorateRow(row, i, "ask"));
  pick(rows);
  if (!list.querySelector("[data-ta-add]")) list.appendChild(addRow());
}

function decorateNext(root: HTMLElement): void {
  const all = [...root.querySelectorAll<HTMLElement>(".options > .option")];
  const rows = all.filter((row) => row.querySelector(".todo-option-badge"));
  if (!rows.length) {
    // A project without todos: the add row still sits before the final "end" option. (Only in that scene; elsewhere the
    // todos have simply not been loaded yet.)
    const end = all[all.length - 1];
    if (params.scene === "next-empty" && end && !end.parentElement?.querySelector("[data-ta-add]")) end.before(addRow());
    return;
  }
  rows.forEach((row, i) => decorateRow(row, i, "next"));
  pick(rows);
  const last = rows[rows.length - 1];
  const list = last.parentElement;
  if (list && !list.querySelector("[data-ta-add]")) last.after(addRow());
}

/** The Stop card: What's next's rows, but "continue" and "end" keep the order the card has today (todos first). */
function decorateStop(root: HTMLElement): void {
  decorateNext(root);
  const add = root.querySelector<HTMLElement>("[data-ta-add]");
  const list = add?.parentElement;
  if (!add || !list || list.dataset.taStop) return;
  const statics = [...list.querySelectorAll<HTMLElement>(":scope > .option")].filter((row) => !row.querySelector(".todo-option-badge"));
  if (statics.length < 2) return;
  list.dataset.taStop = "1";
  add.after(statics[0]);
  [...list.querySelectorAll<HTMLElement>(":scope > .option")].forEach((row, i) => {
    const hotkey = row.querySelector(".opt-sc");
    if (hotkey) hotkey.textContent = `⌘${i + 1}`;
  });
}

let observer: MutationObserver | undefined;
let blurred = false;

function run(): void {
  const root = document.querySelector<HTMLElement>(".rp-main");
  if (!root) return;
  observer?.disconnect();
  try {
    if (params.scene === "stop-now") return;
    if (params.scene === "ask-empty") decorateEmpty(root);
    else if (params.scene.startsWith("ask")) decorateAsk(root);
    else if (params.scene === "stop") decorateStop(root);
    else if (params.scene.startsWith("next")) decorateNext(root);
    else return;
    // While the add field is being typed in, the answer box is not the focused one.
    if (spec.add === "editing" && !blurred && root.querySelector("[data-ta-add]")) {
      blurred = true;
      (document.activeElement as HTMLElement | null)?.blur();
    }
    // `scroll=end` shows the bottom of the content, where the lists are, for a window that is shorter than the page.
    if (params.scroll && root.querySelector("[data-ta-add]")) {
      const content = root.querySelector<HTMLElement>(".popup-main > .content");
      if (content) content.scrollTop = content.scrollHeight;
    }
  } finally {
    observer?.observe(document.body, { childList: true, subtree: true });
  }
}

onMounted(() => {
  observer = new MutationObserver(run);
  observer.observe(document.body, { childList: true, subtree: true });
  run();
});
onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div class="rp-stage">
    <div class="rp-window" data-platform="mac" :style="windowStyle">
      <div class="rp-body">
        <div class="rp-main"><PopupView /></div>
      </div>
      <div class="rp-lights" aria-hidden="true"><i></i><i></i><i></i></div>
    </div>
  </div>
</template>
