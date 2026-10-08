import en from "./i18n/en";
import zh from "./i18n/zh";
import { enhanceAttachmentMermaid } from "./lib/attachmentMermaid";
import { renderMermaid } from "./lib/mermaid";

const root = document.querySelector<HTMLElement>(".markdown-body");
if (root) {
  const sourceHtml = root.innerHTML;
  const common = document.documentElement.lang === "zh" ? zh.common : en.common;
  const appearance = window.matchMedia("(prefers-color-scheme: dark)");
  let dispose: (() => void) | undefined;
  const redraw = () => {
    dispose?.();
    root.innerHTML = sourceHtml;
    dispose = enhanceAttachmentMermaid(root, {
      theme: appearance.matches ? "dark" : "light",
      render: renderMermaid,
      labels: { ...common.mermaid, copy: common.copyCode, copied: common.copied },
    }).dispose;
  };
  redraw();
  if (appearance.addEventListener) appearance.addEventListener("change", redraw);
  else appearance.addListener(redraw);
  window.addEventListener("pagehide", () => dispose?.(), { once: true });
}
