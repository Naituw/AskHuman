import MarkdownIt from "markdown-it";

const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: true,
});

/** Labels for the per-code-block copy button (localized by the caller). */
export interface MarkdownOptions {
  copyLabel?: string;
  copiedLabel?: string;
}

// Copy / done icons reused for the code-block copy button (stroke-based, theme-aware).
const COPY_ICON =
  '<svg class="cc-ico cc-copy" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
const CHECK_ICON =
  '<svg class="cc-ico cc-check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>';

// Wrap a rendered <pre>…</pre> code block so it can host a hover copy button.
// The raw text is read from the DOM at click time (see handleCodeCopyClick), so
// nothing extra needs to be embedded here beyond the localized labels.
function wrapCodeBlock(rendered: string, env: unknown): string {
  const e = (env ?? {}) as MarkdownOptions;
  const copy = md.utils.escapeHtml(e.copyLabel ?? "Copy");
  const copied = md.utils.escapeHtml(e.copiedLabel ?? "Copied");
  const button =
    `<button class="code-copy" type="button" title="${copy}" aria-label="${copy}"` +
    ` data-copy="${copy}" data-copied="${copied}">${COPY_ICON}${CHECK_ICON}</button>`;
  return `<div class="code-block">${rendered}${button}</div>`;
}

const defaultFence = md.renderer.rules.fence?.bind(md.renderer);
md.renderer.rules.fence = (tokens, idx, options, env, self) => {
  const rendered = defaultFence
    ? defaultFence(tokens, idx, options, env, self)
    : self.renderToken(tokens, idx, options);
  return wrapCodeBlock(rendered, env);
};

const defaultCodeBlock = md.renderer.rules.code_block?.bind(md.renderer);
md.renderer.rules.code_block = (tokens, idx, options, env, self) => {
  const rendered = defaultCodeBlock
    ? defaultCodeBlock(tokens, idx, options, env, self)
    : self.renderToken(tokens, idx, options);
  return wrapCodeBlock(rendered, env);
};

export function renderMarkdown(source: string, opts?: MarkdownOptions): string {
  return md.render(source, { ...opts });
}

// Delegated click handler for code-block copy buttons. Returns true when the
// click hit a copy button (so callers can stop further handling). Reads the
// code text from the sibling <code> and shows a brief "copied" state.
export function handleCodeCopyClick(e: MouseEvent): boolean {
  const target = e.target as HTMLElement | null;
  const btn = target?.closest?.(".code-copy") as HTMLElement | null;
  if (!btn) return false;
  e.preventDefault();
  e.stopPropagation();
  const code = btn.closest(".code-block")?.querySelector("code");
  const text = code?.textContent ?? "";

  const showCopied = () => {
    btn.classList.add("copied");
    const copied = btn.getAttribute("data-copied");
    if (copied) btn.setAttribute("title", copied);
    window.setTimeout(() => {
      btn.classList.remove("copied");
      const copy = btn.getAttribute("data-copy");
      if (copy) btn.setAttribute("title", copy);
    }, 1500);
  };

  navigator.clipboard.writeText(text).then(showCopied).catch(() => {
    /* Clipboard unavailable: ignore silently. */
  });
  return true;
}
