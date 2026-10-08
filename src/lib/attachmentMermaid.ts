import type { MermaidDocument } from "./mermaid";
import { loadMermaidAdapter } from "./mermaidLoader";
import { MERMAID_MAX_DIAGRAMS, MERMAID_MAX_TEXT_SIZE, mermaidFitScale } from "./mermaidLimits";
import { clearAtomicFindText, setAtomicFindText } from "./findInDom";

export interface AttachmentMermaidLabels {
  copy: string;
  copied: string;
  diagram: string;
  showSource: string;
  showDiagram: string;
  rendering: string;
  failed: string;
  tooLarge: string;
  tooMany: string;
}

const CSS = `
.attachment-mermaid { margin: 0 0 1em; }
.attachment-mermaid pre { margin-bottom: 0; }
.am-canvas { overflow-x: auto; overflow-y: hidden; padding: 12px 0; }
.am-shell { position: relative; margin: 0 auto; }
.am-frame { display: block; position: absolute; top: 0; left: 0; border: 0; transform-origin: top left; pointer-events: none; }
.am-toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin: 6px 0; font: 12px/1.5 -apple-system, BlinkMacSystemFont, sans-serif; }
.am-toolbar button { font: inherit; color: inherit; background: transparent; border: 1px solid; border-color: color-mix(in srgb, currentColor 25%, transparent); border-radius: 5px; padding: 2px 8px; cursor: pointer; }
.am-toolbar button:focus-visible { outline: 2px solid currentColor; outline-offset: 2px; }
.am-status { opacity: .7; }
.am-canvas[hidden], .attachment-mermaid pre[hidden] { display: none; }
@media print { .am-toolbar { display: none; } .am-canvas { overflow: visible; } }
`;

/** Enhance only backend-marked fences; the surrounding HTML retains its URL policy. */
export function enhanceAttachmentMermaid(
  root: HTMLElement,
  options: {
    theme: "light" | "dark";
    labels: AttachmentMermaidLabels;
    render?: (source: string, theme: "light" | "dark", fontSize: number) => Promise<MermaidDocument>;
    updated?: () => void;
  },
): { done: Promise<void>; dispose: () => void } {
  let disposed = false;
  const cleanups: Array<() => void> = [];
  const blocks = Array.from(root.querySelectorAll<HTMLElement>(".mermaid-block[data-mermaid-pending]"));
  if (blocks.length && !document.getElementById("attachment-mermaid-styles")) {
    const style = document.createElement("style");
    style.id = "attachment-mermaid-styles";
    style.textContent = CSS;
    document.head.appendChild(style);
  }
  const render = options.render ?? (async (source, theme, fontSize) =>
    (await loadMermaidAdapter()).renderMermaid(source, theme, fontSize));
  const labels = options.labels;
  const notify = () => {
    options.updated?.();
    root.dispatchEvent(new CustomEvent("markdown-content-updated", { bubbles: true }));
  };
  const done = Promise.all(blocks.map(async (block, index) => {
    const pre = block.querySelector<HTMLElement>("pre");
    const source = pre?.querySelector("code")?.textContent;
    if (!pre || source === undefined || source === null) return;
    block.classList.add("attachment-mermaid");
    const toolbar = document.createElement("div");
    toolbar.className = "am-toolbar";
    toolbar.setAttribute("data-find-skip", "");
    const copy = document.createElement("button");
    copy.type = "button";
    copy.textContent = labels.copy;
    copy.addEventListener("click", () => {
      void navigator.clipboard?.writeText(source).then(() => {
        if (disposed) return;
        copy.textContent = labels.copied;
        const timer = window.setTimeout(() => { copy.textContent = labels.copy; }, 1500);
        cleanups.push(() => window.clearTimeout(timer));
      }).catch(() => {});
    });
    toolbar.appendChild(copy);
    const status = document.createElement("span");
    status.className = "am-status";
    status.setAttribute("role", "status");
    status.textContent = index >= MERMAID_MAX_DIAGRAMS ? labels.tooMany
      : source.length > MERMAID_MAX_TEXT_SIZE ? labels.tooLarge : labels.rendering;
    toolbar.appendChild(status);
    block.appendChild(toolbar);
    if (index >= MERMAID_MAX_DIAGRAMS || source.length > MERMAID_MAX_TEXT_SIZE) {
      block.removeAttribute("data-mermaid-pending");
      return;
    }
    const fontSize = Number.parseFloat(window.getComputedStyle(root).fontSize) || 14;
    const current = () => !disposed && root.contains(block);
    try {
      const rendered = await render(source, options.theme, fontSize);
      if (!current()) return;
      status.remove();
      const canvas = document.createElement("div");
      canvas.className = "am-canvas";
      const shell = document.createElement("div");
      shell.className = "am-shell";
      const iframe = document.createElement("iframe");
      iframe.className = "am-frame";
      iframe.setAttribute("sandbox", "");
      iframe.setAttribute("referrerpolicy", "no-referrer");
      iframe.setAttribute("scrolling", "no");
      iframe.title = rendered.accTitle || rendered.accDescription || labels.diagram;
      iframe.style.width = `${Math.ceil(rendered.width)}px`;
      iframe.style.height = `${Math.ceil(rendered.height)}px`;
      iframe.style.colorScheme = options.theme;
      iframe.src = rendered.documentUrl;
      shell.appendChild(iframe);
      canvas.appendChild(shell);
      block.insertBefore(canvas, pre);
      const resize = () => {
        if (!current()) return;
        const width = canvas.clientWidth || root.clientWidth;
        const scale = width > 0 ? mermaidFitScale(rendered.width, width, fontSize) : 1;
        shell.style.width = `${Math.ceil(rendered.width * scale)}px`;
        shell.style.height = `${Math.ceil(rendered.height * scale)}px`;
        iframe.style.transform = scale === 1 ? "" : `scale(${scale})`;
      };
      resize();
      if (typeof ResizeObserver !== "undefined") {
        const observer = new ResizeObserver(resize);
        observer.observe(canvas);
        cleanups.push(() => observer.disconnect());
      }
      window.addEventListener("resize", resize);
      cleanups.push(() => window.removeEventListener("resize", resize));
      pre.hidden = true;
      pre.setAttribute("data-find-skip", "");
      block.setAttribute("data-find-atomic", "");
      setAtomicFindText(block, rendered.findText);
      cleanups.push(() => clearAtomicFindText(block));
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.textContent = labels.showSource;
      toggle.setAttribute("aria-pressed", "false");
      toggle.addEventListener("click", () => {
        pre.hidden = !pre.hidden;
        canvas.hidden = !pre.hidden;
        toggle.textContent = pre.hidden ? labels.showSource : labels.showDiagram;
        toggle.setAttribute("aria-pressed", String(!pre.hidden));
        if (pre.hidden) {
          pre.setAttribute("data-find-skip", "");
          block.setAttribute("data-find-atomic", "");
          setAtomicFindText(block, rendered.findText);
        } else {
          pre.removeAttribute("data-find-skip");
          block.removeAttribute("data-find-atomic");
          clearAtomicFindText(block);
        }
        if (pre.hidden) resize();
        notify();
      });
      toolbar.appendChild(toggle);
      block.dataset.mermaidRendered = "";
    } catch (error) {
      if (!current()) return;
      status.textContent = typeof error === "object" && error !== null && "code" in error && error.code === "tooLarge"
        ? labels.tooLarge : labels.failed;
      status.setAttribute("role", "alert");
    }
    if (current()) {
      block.removeAttribute("data-mermaid-pending");
      notify();
    }
  })).then(() => {});
  return { done, dispose: () => { disposed = true; for (const cleanup of cleanups.splice(0)) cleanup(); } };
}
