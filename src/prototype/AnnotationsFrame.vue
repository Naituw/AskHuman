<script setup lang="ts">
// Prototype of "select text, write an annotation, and the annotations go to the Agent with the answer" in the popup.
// The real PopupView runs inside a fake window (as in popup-reply-menu.html); what this prototype adds is teleported
// into the real DOM so it sits among the product's own controls:
//  - a small floating button next to a text selection in the Message (or, in a direct ask with no Message and one
//    question, in that question's text: it is then the body of the request),
//  - the editor popover (the quoted passage and the note),
//  - highlights wrapped around annotated passages (numbered, click to edit),
//  - a chip beside Send with the count; it opens the list of annotations (locate / edit / delete).
// `?scene=select|editor|notes|list|edit` draws one fixed state for screenshots; every state can also be reached by hand
// (select text with the mouse). `?shape=message|direct|three` picks the request (a Message and one question; a direct ask;
// a Message and three questions, whose question texts cannot be annotated); `theme=dark`, `platform=win`, `w=`, `h=` as in
// the reply-menu prototype.
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef } from "vue";
import PopupView from "../views/PopupView.vue";
import { readParams } from "./annotationsMock";
import "./annotations.css";

type Scene = "idle" | "select" | "editor" | "notes" | "list" | "edit";
interface Note { id: number; seg: string; start: number; end: number; quote: string; text: string }
interface Pending { seg: string; start: number; end: number; quote: string; overlap: number | null }
interface EditorState { id: number | null; seg: string; start: number; end: number; quote: string; draft: string; x: number; y: number }

const params = readParams();
const mac = params.platform === "mac";
const scene = (new URLSearchParams(location.search).get("scene") ?? "idle") as Scene;
const windowStyle = {
  "--rp-win-w": `${params.width}px`,
  "--rp-win-h": `${params.height + (mac ? 0 : 32)}px`,
};

const windowEl = ref<HTMLElement | null>(null);
const mainEl = shallowRef<HTMLElement | null>(null);
const footerEl = shallowRef<HTMLElement | null>(null);
const taEl = ref<HTMLTextAreaElement | null>(null);

const notes = ref<Note[]>([]);
const pending = ref<Pending | null>(null);
const fab = ref<{ x: number; y: number } | null>(null);
const editor = ref<EditorState | null>(null);
const listOpen = ref(false);
let nextId = 1;

// ---- DOM model: the text of a segment is the concatenation of its text nodes ------------------------------------
// Only the Message carries annotations and, in a direct ask (no Message, one question), that question's text: it is then
// the body of the request. A question's text beside a Message or among several questions does not: the user writes what
// they want to say in the answer itself.
function segEls(): HTMLElement[] {
  const all = Array.from(document.querySelectorAll<HTMLElement>(".an-main [data-find-seg]"));
  const hasMessage = all.some((el) => el.dataset.findSeg === "message");
  const questionTexts = all.filter((el) => /^q-\d+-msg$/.test(el.dataset.findSeg ?? ""));
  const direct = !hasMessage && questionTexts.length === 1;
  return all.filter((el) => el.dataset.findSeg === "message" || (direct && questionTexts.includes(el)));
}
function segEl(seg: string): HTMLElement | null {
  return segEls().find((el) => el.dataset.findSeg === seg) ?? null;
}
function segOf(node: Node | null): HTMLElement | null {
  const el = node && node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node?.parentElement ?? null;
  const seg = el?.closest<HTMLElement>("[data-find-seg]") ?? null;
  return seg && segEls().includes(seg) ? seg : null;
}
function textNodes(root: HTMLElement): Text[] {
  const out: Text[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (!(n.parentElement?.closest("[data-find-skip]"))) out.push(n as Text);
  }
  return out;
}
function textOf(root: HTMLElement): string {
  return textNodes(root).map((n) => n.data).join("");
}
function offsetIn(root: HTMLElement, node: Node, offset: number): number {
  const r = document.createRange();
  r.selectNodeContents(root);
  r.setEnd(node, offset);
  return r.toString().length;
}
function rangeFor(root: HTMLElement, start: number, end: number): Range {
  const range = document.createRange();
  let pos = 0;
  let startSet = false;
  for (const node of textNodes(root)) {
    const len = node.data.length;
    if (!startSet && start < pos + len) { range.setStart(node, start - pos); startSet = true; }
    if (startSet && end <= pos + len) { range.setEnd(node, end - pos); break; }
    pos += len;
  }
  return range;
}
/** Wrap [start, end) of a segment's text in `<mark>`s (one per text node it touches); returns them in order. */
function wrap(root: HTMLElement, start: number, end: number, cls: string, attrs: Record<string, string> = {}): HTMLElement[] {
  const marks: HTMLElement[] = [];
  let pos = 0;
  for (const node of textNodes(root)) {
    const len = node.data.length;
    const from = Math.max(start - pos, 0);
    const to = Math.min(end - pos, len);
    if (from < to) {
      const mid = node.splitText(from);
      mid.splitText(to - from);
      const mark = document.createElement("mark");
      mark.className = cls;
      for (const [k, v] of Object.entries(attrs)) mark.setAttribute(k, v);
      mid.parentNode?.replaceChild(mark, mid);
      mark.appendChild(mid);
      marks.push(mark);
    }
    pos += len;
    if (pos >= end) break;
  }
  return marks;
}
function unwrap(selector: string): void {
  for (const mark of Array.from(document.querySelectorAll<HTMLElement>(`.an-main ${selector}`))) {
    const parent = mark.parentNode;
    if (!parent) continue;
    while (mark.firstChild) parent.insertBefore(mark.firstChild, mark);
    parent.removeChild(mark);
    parent.normalize();
  }
}

// ---- Notes -------------------------------------------------------------------------------------------------------
function order(seg: string): number {
  return segEls().findIndex((el) => el.dataset.findSeg === seg);
}
const sorted = computed(() => [...notes.value].sort((a, b) => order(a.seg) - order(b.seg) || a.start - b.start));
const excerpt = (text: string, max = 60) => (text.length > max ? `${text.slice(0, max)}…` : text);

function renderMarks(): void {
  unwrap("mark.an-mark");
  sorted.value.forEach((note, i) => {
    const el = segEl(note.seg);
    if (!el) return;
    const marks = wrap(el, note.start, note.end, "an-mark", { "data-note": String(note.id) });
    const last = marks[marks.length - 1];
    if (last) { last.classList.add("an-last"); last.setAttribute("data-n", String(i + 1)); }
  });
}

// ---- Selection -> floating button ----------------------------------------------------------------------------------
function place(rect: DOMRect, width: number, height: number, prefer: "above" | "below"): { x: number; y: number } {
  const win = windowEl.value!.getBoundingClientRect();
  let x = rect.left + rect.width / 2 - width / 2 - win.left;
  x = Math.max(12, Math.min(x, win.width - width - 12));
  let y = prefer === "above" ? rect.top - win.top - height - 8 : rect.bottom - win.top + 8;
  if (y < 64) y = rect.bottom - win.top + 8;
  if (y + height > win.height - 70) y = rect.top - win.top - height - 8;
  return { x, y };
}
function checkSelection(): void {
  if (editor.value) return;
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) { fab.value = null; pending.value = null; return; }
  const range = sel.getRangeAt(0);
  const a = segOf(range.startContainer);
  const b = segOf(range.endContainer);
  if (!a || a !== b) { fab.value = null; pending.value = null; return; }
  const start = offsetIn(a, range.startContainer, range.startOffset);
  const end = offsetIn(a, range.endContainer, range.endOffset);
  const quote = range.toString().replace(/\s+/g, " ").trim();
  if (!quote) { fab.value = null; pending.value = null; return; }
  const seg = a.dataset.findSeg!;
  const hit = notes.value.find((n) => n.seg === seg && n.start < end && start < n.end);
  pending.value = { seg, start, end, quote, overlap: hit?.id ?? null };
  fab.value = place(range.getBoundingClientRect(), 98, 26, "above");
}
function onMouseUp(e: MouseEvent): void {
  if ((e.target as HTMLElement | null)?.closest(".an-editor, .an-fab, .an-anchor")) return;
  window.setTimeout(checkSelection, 0);
}
function onSelectionChange(): void {
  if (editor.value) return;
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed) { fab.value = null; pending.value = null; }
}
function onScroll(): void {
  fab.value = null;
  if (editor.value) cancelEditor();
}

// ---- Editor --------------------------------------------------------------------------------------------------------
function openEditorFromSelection(): void {
  const p = pending.value;
  if (!p) return;
  if (p.overlap !== null) {
    const note = notes.value.find((n) => n.id === p.overlap);
    window.getSelection()?.removeAllRanges();
    fab.value = null;
    if (note) openEditorFor(note);
    return;
  }
  const el = segEl(p.seg);
  if (!el) return;
  const rect = rangeFor(el, p.start, p.end).getBoundingClientRect();
  wrap(el, p.start, p.end, "an-draft");
  editor.value = { id: null, seg: p.seg, start: p.start, end: p.end, quote: p.quote, draft: "", ...place(rect, 320, 176, "below") };
  window.getSelection()?.removeAllRanges();
  fab.value = null;
  void nextTick(() => taEl.value?.focus());
}
function openEditorFor(note: Note): void {
  const mark = document.querySelector<HTMLElement>(`.an-main mark.an-mark[data-note="${note.id}"]`);
  if (!mark) return;
  listOpen.value = false;
  editor.value = { id: note.id, seg: note.seg, start: note.start, end: note.end, quote: note.quote, draft: note.text, ...place(mark.getBoundingClientRect(), 320, 176, "below") };
  void nextTick(() => taEl.value?.focus());
}
function cancelEditor(): void {
  unwrap("mark.an-draft");
  editor.value = null;
}
function commitEditor(): void {
  const e = editor.value;
  if (!e || !e.draft.trim()) return;
  unwrap("mark.an-draft");
  if (e.id === null) notes.value.push({ id: nextId++, seg: e.seg, start: e.start, end: e.end, quote: e.quote, text: e.draft.trim() });
  else { const n = notes.value.find((x) => x.id === e.id); if (n) n.text = e.draft.trim(); }
  editor.value = null;
  renderMarks();
}
function removeNote(id: number): void {
  notes.value = notes.value.filter((n) => n.id !== id);
  editor.value = null;
  if (!notes.value.length) listOpen.value = false;
  renderMarks();
}
function onEditorKey(e: KeyboardEvent): void {
  if (e.key === "Escape") { e.preventDefault(); cancelEditor(); }
  else if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !e.isComposing) { e.preventDefault(); commitEditor(); }
}
function reveal(note: Note): void {
  const marks = Array.from(document.querySelectorAll<HTMLElement>(`.an-main mark.an-mark[data-note="${note.id}"]`));
  marks[0]?.scrollIntoView({ block: "center", behavior: "smooth" });
  for (const m of marks) { m.classList.remove("an-flash"); void m.offsetWidth; m.classList.add("an-flash"); }
}
function onDocClick(e: MouseEvent): void {
  const mark = (e.target as HTMLElement | null)?.closest<HTMLElement>("mark.an-mark");
  if (mark && !editor.value) {
    const note = notes.value.find((n) => n.id === Number(mark.dataset.note));
    if (note) openEditorFor(note);
  }
}

// ---- Fixed scenes (screenshots) ------------------------------------------------------------------------------------
const SAMPLES =
  params.shape === "direct"
    ? {
        a: { seg: "q-0-msg", phrase: "少 3 个索引", text: "是哪 3 个？为什么没建？" },
        b: { seg: "q-0-msg", phrase: "要不要现在补齐索引", text: "补索引要锁表吗？大概多久？" },
        c: { seg: "q-0-msg", phrase: "查询在 staging 上慢了约 18%", text: "是哪几个查询？有慢查询日志吗？" },
      }
    : {
        a: { seg: "message", phrase: "需要同时调整三处表单", text: "是哪三处？把文件名列一下，我想先看影响面。" },
        b: { seg: "message", phrase: "回归测试也已补充", text: "补在哪个文件里？覆盖 Safari 15 了吗？" },
        c: { seg: "message", phrase: "Chrome 与 Firefox 下表现一致", text: "具体是哪两个版本？" },
      };
function sample(s: { seg: string; phrase: string; text: string }): Note | null {
  const el = segEl(s.seg);
  if (!el) return null;
  const start = textOf(el).indexOf(s.phrase);
  if (start < 0) return null;
  return { id: nextId++, seg: s.seg, start, end: start + s.phrase.length, quote: s.phrase, text: s.text };
}
function addSamples(...keys: Array<keyof typeof SAMPLES>): void {
  for (const k of keys) { const n = sample(SAMPLES[k]); if (n) notes.value.push(n); }
  renderMarks();
}
const sleep = (ms: number) => new Promise((r) => window.setTimeout(r, ms));
async function waitFor(ok: () => boolean, ms = 6000): Promise<void> {
  const until = Date.now() + ms;
  while (!ok() && Date.now() < until) await sleep(80);
}
async function runScene(): Promise<void> {
  if (scene === "idle") return;
  await waitFor(() => segEls().length > 0 && !!document.querySelector(".an-main .footer"));
  await sleep(250);
  const a = sample(SAMPLES.a);
  if (scene === "select" && a) {
    const el = segEl(a.seg)!;
    wrap(el, a.start, a.end, "an-sel");
    pending.value = { seg: a.seg, start: a.start, end: a.end, quote: a.quote, overlap: null };
    fab.value = place(rangeFor(el, a.start, a.end).getBoundingClientRect(), 98, 26, "above");
  } else if (scene === "editor" && a) {
    const el = segEl(a.seg)!;
    wrap(el, a.start, a.end, "an-draft");
    editor.value = { id: null, seg: a.seg, start: a.start, end: a.end, quote: a.quote, draft: a.text, ...place(rangeFor(el, a.start, a.end).getBoundingClientRect(), 320, 176, "below") };
  } else if (scene === "notes") {
    addSamples("a", "b");
  } else if (scene === "list") {
    addSamples("a", "b", "c");
    await nextTick();
    listOpen.value = true;
  } else if (scene === "edit") {
    addSamples("a", "b");
    await nextTick();
    if (notes.value[0]) openEditorFor(notes.value[0]);
  }
}

// The popup renders after `popup_init` resolves, so the landmarks are tracked as live element refs.
let observer: MutationObserver | undefined;
function probe(): void {
  const main = document.querySelector<HTMLElement>(".an-main .popup-main");
  mainEl.value = main;
  footerEl.value = main?.querySelector<HTMLElement>(":scope > .footer") ?? null;
}
onMounted(() => {
  observer = new MutationObserver(probe);
  observer.observe(document.body, { childList: true, subtree: true });
  probe();
  window.addEventListener("mouseup", onMouseUp);
  document.addEventListener("selectionchange", onSelectionChange);
  document.addEventListener("click", onDocClick);
  window.addEventListener("scroll", onScroll, true);
  void runScene();
});
onBeforeUnmount(() => {
  observer?.disconnect();
  window.removeEventListener("mouseup", onMouseUp);
  document.removeEventListener("selectionchange", onSelectionChange);
  document.removeEventListener("click", onDocClick);
  window.removeEventListener("scroll", onScroll, true);
});
</script>

<template>
  <div class="an-stage" :data-scene="scene" @click="listOpen = false">
    <div ref="windowEl" class="an-window" :data-platform="params.platform" :style="windowStyle">
      <div v-if="!mac" class="an-winbar">
        <span class="an-win-icon"></span>
        <span class="an-win-title">AskHuman</span>
      </div>
      <div class="an-body">
        <div class="an-main"><PopupView /></div>
      </div>
      <div v-if="mac" class="an-lights" aria-hidden="true"><i></i><i></i><i></i></div>

      <!-- The floating button beside a selection: a click on it must not collapse the selection first. -->
      <button v-if="fab && !editor" class="an-fab" type="button" :style="{ left: `${fab.x}px`, top: `${fab.y}px` }" @mousedown.prevent @click.stop="openEditorFromSelection">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M21 11.5a8.4 8.4 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.4 8.4 0 0 1-3.8-.9L3 21l1.9-5.7a8.4 8.4 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.4 8.4 0 0 1 3.8-.9h.5a8.5 8.5 0 0 1 8 8z" />
          <path d="M12 8.5v6M9 11.5h6" />
        </svg>
        {{ pending?.overlap ? "编辑附注" : "添加附注" }}
      </button>

      <!-- The editor: a modal popover (its keys never reach the popup's own shortcuts). -->
      <div v-if="editor" class="an-editor" :style="{ left: `${editor.x}px`, top: `${editor.y}px` }" role="dialog" aria-label="附注" @mousedown.stop @click.stop @keydown.stop="onEditorKey">
        <div class="an-quote">{{ excerpt(editor.quote, 90) }}</div>
        <textarea ref="taEl" v-model="editor.draft" class="an-textarea" rows="3" placeholder="写下你的疑问或意见…" @paste.stop></textarea>
        <div class="an-editor-foot">
          <button v-if="editor.id !== null" class="an-delete" type="button" @click="removeNote(editor.id)">删除</button>
          <span v-else class="an-hint"><kbd class="an-kbd">⌘↵</kbd> 添加</span>
          <span class="an-spacer"></span>
          <button class="btn" type="button" @click="cancelEditor">取消</button>
          <button class="btn btn-primary" type="button" :disabled="!editor.draft.trim()" @click="commitEditor">{{ editor.id === null ? "添加" : "保存" }}</button>
        </div>
      </div>
    </div>

    <!-- Footer: the chip sits beside Send (after the quick-reply menu), only while there are annotations. -->
    <Teleport v-if="footerEl && notes.length" :to="footerEl">
      <div class="qr-anchor an-anchor" @click.stop>
        <button class="btn qr-trigger an-chip" :class="{ open: listOpen }" type="button" aria-haspopup="menu" :aria-expanded="listOpen" @click="listOpen = !listOpen">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M21 11.5a8.4 8.4 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.4 8.4 0 0 1-3.8-.9L3 21l1.9-5.7a8.4 8.4 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.4 8.4 0 0 1 3.8-.9h.5a8.5 8.5 0 0 1 8 8z" />
          </svg>
          附注
          <span class="an-count">{{ notes.length }}</span>
        </button>
        <div v-if="listOpen" class="qr-menu an-list" role="menu">
          <div class="qr-head">
            <span>附注 · {{ notes.length }} 条</span>
            <span class="qr-head-hint">提交时随回答一起发给 Agent</span>
          </div>
          <div v-for="(n, i) in sorted" :key="n.id" class="an-row" role="menuitem" tabindex="0" @click="reveal(n)">
            <span class="an-num">{{ i + 1 }}</span>
            <span class="an-row-body">
              <span class="an-row-quote">“{{ excerpt(n.quote, 40) }}”</span>
              <span class="an-row-text">{{ n.text }}</span>
            </span>
            <span class="an-row-actions">
              <button class="an-icon-btn" type="button" title="编辑" aria-label="编辑" @click.stop="openEditorFor(n)">
                <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" /></svg>
              </button>
              <button class="an-icon-btn" type="button" title="删除" aria-label="删除" @click.stop="removeNote(n.id)">
                <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true"><line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /><line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /></svg>
              </button>
            </span>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>
