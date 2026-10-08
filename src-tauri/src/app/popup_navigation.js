(() => {
  // WebView2 also injects initialization scripts into frames. Leave sandboxed diagrams alone.
  if (window !== window.top) return;
  const preventLinkNavigation = event => {
    const element = event.target instanceof Element ? event.target : event.target?.parentElement;
    if (element?.closest("a[href]")) event.preventDefault();
  };
  // Keep Vue's delegated handlers running so they can open supported destinations externally.
  document.addEventListener("click", preventLinkNavigation, true);
  document.addEventListener("auxclick", preventLinkNavigation, true);
})();
