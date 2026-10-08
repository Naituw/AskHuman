import { describe, expect, it, vi } from "vitest";
import script from "../../src-tauri/src/app/popup_navigation.js?raw";

describe("popup document navigation protection", () => {
  it.each(["click", "auxclick"])("cancels an unhandled %s even when a link stops bubbling", type => {
    const root = document.createElement("div");
    root.innerHTML = '<a href="/Users/test/file.md"><span>file</span></a><button>button</button>';
    document.body.appendChild(root);
    const listen = vi.spyOn(document, "addEventListener");
    new Function("window", "document", "Element", script)(window, document, Element);
    const registrations = [...listen.mock.calls];
    listen.mockRestore();
    try {
      const anchor = root.querySelector("a")!;
      const delegated = vi.fn((event: Event) => event.stopPropagation());
      anchor.addEventListener(type, delegated);
      const click = new MouseEvent(type, { bubbles: true, cancelable: true });
      anchor.querySelector("span")!.dispatchEvent(click);
      expect(click.defaultPrevented).toBe(true);
      expect(delegated).toHaveBeenCalledOnce();
      const buttonClick = new MouseEvent(type, { bubbles: true, cancelable: true });
      root.querySelector("button")!.dispatchEvent(buttonClick);
      expect(buttonClick.defaultPrevented).toBe(false);
    } finally {
      for (const [name, listener, options] of registrations) document.removeEventListener(name, listener, options);
      root.remove();
    }
  });

  it("does not install listeners into Mermaid subframes", () => {
    const iframe = document.createElement("iframe");
    document.body.appendChild(iframe);
    try {
      const frame = iframe.contentWindow!;
      const listen = vi.spyOn(frame.document, "addEventListener");
      new Function("window", "document", "Element", script)(frame, frame.document, Element);
      expect(listen).not.toHaveBeenCalled();
      listen.mockRestore();
    } finally { iframe.remove(); }
  });
});
