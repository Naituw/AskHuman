<script setup lang="ts">
// The "annotations in the popup" page. The real PopupView, with the product's own annotation layer, footer chip and list,
// runs inside a fake window on a mocked backend; this page draws nothing of the feature itself. It picks the request
// (`?shape=message|direct|three`: a Message and one question; a direct ask with no Message; a Message and three
// questions) and, for `?scene=`, drives the real UI the way a person does (select a passage, press the button, type,
// add) so a state can be reached on load, e.g. for a screenshot:
//   select  a passage is selected and the button stands beside it
//   editor  the editor is open on a passage with a note typed
//   notes   two notes are added: highlights with numbers and the chip beside Send
//   list    three notes are added and the list is open
//   edit    two notes are added and the first one is open for editing
// Everything can also be done by hand with the mouse; `theme=dark`, `platform=win`, `w=`, `h=` as in the reply-menu page.
import { onMounted } from "vue";
import PopupView from "../views/PopupView.vue";
import { readParams } from "./annotationsMock";
import "./annotations.css";

type Scene = "idle" | "select" | "editor" | "notes" | "list" | "edit";
interface Sample { phrase: string; text: string }

const params = readParams();
const mac = params.platform === "mac";
const scene = (new URLSearchParams(location.search).get("scene") ?? "idle") as Scene;
const windowStyle = {
  "--rp-win-w": `${params.width}px`,
  "--rp-win-h": `${params.height + (mac ? 0 : 32)}px`,
};

// What to select and write, per request shape (the `three` shape annotates its Message, like `message`).
const SAMPLES: Record<"a" | "b" | "c", Sample> =
  params.shape === "direct"
    ? {
        a: { phrase: "少 3 个索引", text: "是哪 3 个？为什么没建？" },
        b: { phrase: "要不要现在补齐索引", text: "补索引要锁表吗？大概多久？" },
        c: { phrase: "查询在 staging 上慢了约 18%", text: "是哪几个查询？有慢查询日志吗？" },
      }
    : params.shape === "three"
      ? {
          a: { phrase: "独立会话表更利于后续做多端登录管理", text: "多端登录的需求现在确定了吗？" },
          b: { phrase: "线上还有约 1.2 万条旧会话", text: "迁移要停机吗？" },
          c: { phrase: "边界与回归还没补", text: "先补哪几条？" },
        }
      : {
          a: { phrase: "需要同时调整三处表单", text: "是哪三处？把文件名列一下，我想先看影响面。" },
          b: { phrase: "回归测试也已补充", text: "补在哪个文件里？覆盖 Safari 15 了吗？" },
          c: { phrase: "Chrome 与 Firefox 下表现一致", text: "具体是哪两个版本？" },
        };

const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));
async function waitFor<T>(find: () => T | null | undefined, ms = 6000): Promise<T | null> {
  const until = Date.now() + ms;
  for (;;) {
    const found = find();
    if (found) return found;
    if (Date.now() > until) return null;
    await sleep(60);
  }
}
const $ = <T extends Element>(selector: string) => document.querySelector<T>(selector);

/** Select a passage of the annotatable text as a mouse drag would: press, the selection forms, release. */
async function select(phrase: string): Promise<boolean> {
  const root = await waitFor(() => $<HTMLElement>("[data-annotation-root]"));
  if (!root) return false;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const index = (node as Text).data.indexOf(phrase);
    if (index < 0) continue;
    const range = document.createRange();
    range.setStart(node, index);
    range.setEnd(node, index + phrase.length);
    root.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    document.dispatchEvent(new MouseEvent("pointerup", { bubbles: true }));
    return !!(await waitFor(() => $(".an-fab")));
  }
  return false;
}

async function write(text: string): Promise<void> {
  const box = await waitFor(() => $<HTMLTextAreaElement>(".an-editor .an-textarea"));
  if (!box) return;
  box.value = text;
  box.dispatchEvent(new Event("input", { bubbles: true }));
  await sleep(60);
}

async function openEditorOn(sample: Sample): Promise<void> {
  if (!(await select(sample.phrase))) return;
  $<HTMLButtonElement>(".an-fab")?.click();
  await write(sample.text);
}

async function addNote(sample: Sample): Promise<void> {
  await openEditorOn(sample);
  $<HTMLButtonElement>(".an-editor .btn-primary")?.click();
  await waitFor(() => !$(".an-editor"));
  await sleep(60);
}

async function runScene(): Promise<void> {
  if (scene === "idle") return;
  await waitFor(() => $("[data-annotation-root]") && $(".popup-main > .footer"));
  await sleep(250);
  const { a, b, c } = SAMPLES;
  if (scene === "select") await select(a.phrase);
  else if (scene === "editor") await openEditorOn(a);
  else if (scene === "notes") { await addNote(a); await addNote(b); }
  else if (scene === "list") {
    await addNote(a); await addNote(b); await addNote(c);
    $<HTMLButtonElement>(".an-chip")?.click();
  } else if (scene === "edit") {
    await addNote(a); await addNote(b);
    $<HTMLElement>("mark.an-mark")?.click();
  }
}

onMounted(() => void runScene());
</script>

<template>
  <div class="an-stage" :data-scene="scene">
    <div class="an-window" :data-platform="params.platform" :style="windowStyle">
      <div v-if="!mac" class="an-winbar">
        <span class="an-win-icon"></span>
        <span class="an-win-title">AskHuman</span>
      </div>
      <div class="an-body">
        <div class="an-main"><PopupView /></div>
      </div>
      <div v-if="mac" class="an-lights" aria-hidden="true"><i></i><i></i><i></i></div>
    </div>
  </div>
</template>
