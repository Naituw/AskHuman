// A small CSS cascade for component tests. jsdom matches selectors, but it neither tracks :hover nor sorts competing rules
// by specificity, so a test that depends on WHICH rule wins (a state look that a more specific rule hides, a layout that
// only a platform or form class changes) cannot ask jsdom. This resolves the winner itself: real selector matching against
// the component's own stylesheet, then specificity, then source order.
//
// It knows flat rules only. The stops of a keyframes block are read as harmless rules that nothing matches, and the
// stylesheets this is used on hold no at-rule that a checked property depends on (the tests would be wrong if one did).

interface Rule { selector: string; order: number; declarations: Record<string, string> }

/** The contents of the single `<style>` block of a single-file component's source. */
export function styleBlock(component: string): string {
  const blocks = [...component.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)];
  if (blocks.length !== 1) throw new Error(`expected exactly one <style> block, found ${blocks.length}`);
  return blocks[0][1];
}

function splitSelectors(list: string): string[] {
  const parts: string[] = [];
  let depth = 0, start = 0;
  for (let i = 0; i < list.length; i++) {
    if (list[i] === "(" || list[i] === "[") depth++;
    else if (list[i] === ")" || list[i] === "]") depth--;
    else if (list[i] === "," && depth === 0) { parts.push(list.slice(start, i)); start = i + 1; }
  }
  parts.push(list.slice(start));
  return parts.map(part => part.trim()).filter(Boolean);
}

function parse(text: string): Rule[] {
  const stripped = text.replace(/\/\*[\s\S]*?\*\//g, "");
  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  const rules: Rule[] = [];
  let order = 0;
  for (let match = pattern.exec(stripped); match; match = pattern.exec(stripped)) {
    const declarations: Record<string, string> = {};
    for (const declaration of match[2].split(";")) {
      const colon = declaration.indexOf(":");
      if (colon > 0) declarations[declaration.slice(0, colon).trim()] = declaration.slice(colon + 1).trim();
    }
    for (const selector of splitSelectors(match[1])) rules.push({ selector, order: order++, declarations });
  }
  return rules;
}

function specificity(selector: string): number {
  const flat = selector.replace(/:not\(([^)]*)\)/g, " $1 ");
  const ids = (flat.match(/#[\w-]+/g) ?? []).length;
  const classes = (flat.match(/\.[\w-]+|\[[^\]]*\]|:(?!:)[\w-]+/g) ?? []).length;
  const types = (flat.replace(/\[[^\]]*\]|[.#:][\w-]+/g, " ").match(/(?:^|[\s>+~])[a-z][\w-]*/gi) ?? []).length;
  return ids * 10000 + classes * 100 + types;
}

/**
 * The value a property gets from `css`, as a function of the element. `states` turns the pseudo-classes jsdom does not
 * track into classes the fixture sets by hand (`{ ":hover": ".is-hovered" }`): selectors are rewritten before matching and
 * the stand-in counts as one class, exactly like the pseudo-class it replaces.
 */
export function createCascade(css: string, states: Record<string, string> = {}) {
  const rules = parse(css);
  const rewrite = (selector: string) => Object.entries(states).reduce((text, [pseudo, stand]) => text.replaceAll(pseudo, stand), selector);
  return function cascade(element: Element, property: string): string | undefined {
    let winner: { specificity: number; order: number; value: string } | undefined;
    for (const rule of rules) {
      const value = rule.declarations[property];
      if (value === undefined) continue;
      const selector = rewrite(rule.selector);
      let matches = false;
      try { matches = element.matches(selector); } catch { /* a pseudo-element or something jsdom cannot match */ }
      if (!matches) continue;
      const candidate = { specificity: specificity(selector), order: rule.order, value };
      if (!winner || candidate.specificity > winner.specificity || (candidate.specificity === winner.specificity && candidate.order > winner.order)) winner = candidate;
    }
    return winner?.value;
  };
}
