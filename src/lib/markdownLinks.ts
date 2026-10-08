import { openPath } from "./ipc";

export type MarkdownLink =
  | { kind: "open"; target: string }
  | { kind: "fragment"; target: string };

/** Resolve the authored href, before the WebView turns file paths into app URLs. */
export function resolveMarkdownLink(href: string, baseDirectory = ""): MarkdownLink | null {
  const value = href.trim();
  if (!value) return null;
  if (/^(https?:|mailto:)/i.test(value) || value.startsWith("//")) {
    try {
      return { kind: "open", target: new URL(value.startsWith("//") ? `https:${value}` : value).href };
    } catch { return null; }
  }
  if (value.startsWith("#")) {
    try { return { kind: "fragment", target: decodeURIComponent(value.slice(1)) }; }
    catch { return null; }
  }

  let path = value.split(/[?#]/, 1)[0];
  if (/^file:/i.test(value)) {
    try {
      const url = new URL(value);
      path = url.host && url.host !== "localhost" ? `\\\\${url.host}${url.pathname.replace(/\//g, "\\")}` : url.pathname;
      if (/^\/[a-z]:\//i.test(path)) path = path.slice(1);
    } catch { return null; }
  } else if (!/^[a-z]:(?:[\\/]|%5c|%2f)/i.test(path) && /^[a-z][a-z\d+.-]*:/i.test(path)) {
    return null;
  }
  // Agent file citations append a line and optional column; they are not part of the filename.
  path = path.replace(/:\d+(?::\d+)?$/, "");
  try { path = decodeURIComponent(path); } catch { return null; }
  if (!path || path.includes("\0")) return null;
  if (!path.startsWith("/") && !/^[a-z]:[\\/]/i.test(path) && !path.startsWith("\\\\")) {
    // A relative file belongs to its request's project, never to the app resource origin.
    if (!baseDirectory) return null;
    const separator = /^[a-z]:[\\/]/i.test(baseDirectory) || baseDirectory.startsWith("\\\\") ? "\\" : "/";
    path = `${baseDirectory.replace(/[\\/]$/, "")}${separator}${path}`;
  }
  return { kind: "open", target: path };
}

/** Always cancel link navigation, including unsupported or malformed destinations. */
export function handleMarkdownLinkClick(event: MouseEvent, baseDirectory = ""): boolean {
  const element = event.target instanceof Element ? event.target : (event.target as Node | null)?.parentElement;
  const anchor = element?.closest("a[href]");
  if (!anchor) return false;
  event.preventDefault();
  event.stopPropagation();
  const link = resolveMarkdownLink(anchor.getAttribute("href") ?? "", baseDirectory);
  if (link?.kind === "open") {
    void openPath(link.target).catch(() => {});
  } else if (link?.kind === "fragment") {
    const root = event.currentTarget;
    if (root instanceof Element) {
      Array.from(root.querySelectorAll<HTMLElement>("[id]")).find(node => node.id === link.target)?.scrollIntoView();
    }
  }
  return true;
}
