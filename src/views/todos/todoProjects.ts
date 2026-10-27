// Sidebar view model of the todos window (spec todo-sync §9): labels, grouping, search, keyboard
// stepping and the "All todos" aggregate. Pure functions only, so the window stays cheap to open
// (everything here is O(projects) string work) and is unit-testable without a DOM.

import type { TodoEntry, TodoGroup, TodoProjectInfo } from "../../lib/types";

/** Key of the "All todos" sidebar row. Project keys are absolute paths, so this cannot collide. */
export const ALL_KEY = "__all__";

/** Rows shown in the "All todos" view before "show more" (spec todo-sync §9.3). */
export const ALL_ROW_LIMIT = 50;

export interface ProjectLabel {
  /** Display name (basename of the project directory). */
  name: string;
  /** Gray suffix telling same-named projects apart; empty when the name is unique. */
  hint: string;
  /** Path for tooltips and the project header, with the home directory shown as `~`. */
  path: string;
}

export interface SidebarRow extends ProjectLabel {
  key: string;
  count: number;
}

export interface SidebarSection {
  id: "withTodos" | "recent";
  rows: SidebarRow[];
}

export interface SidebarModel {
  /** Pending todos over all projects (not narrowed by the search). */
  allCount: number;
  /** Non-empty sections only, in display order. */
  sections: SidebarSection[];
  /** Keyboard order: the "All todos" row followed by every visible project row. */
  navKeys: string[];
  /** Number of project rows after the search filter. */
  rowCount: number;
}

function splitPath(path: string): string[] {
  return path
    .replace(/\\/g, "/")
    .split("/")
    .filter((part) => part.length > 0);
}

export function baseName(path: string): string {
  const parts = splitPath(path);
  return parts[parts.length - 1] ?? path;
}

/** `/Users/me/dev/app` → `~/dev/app` when `home` is `/Users/me`; other paths are unchanged. */
export function abbreviateHome(path: string, home?: string): string {
  if (!home) return path;
  const base = home.replace(/[\\/]+$/, "");
  if (!base) return path;
  if (path === base) return "~";
  const next = path.charAt(base.length);
  if (path.startsWith(base) && (next === "/" || next === "\\")) {
    return `~${path.slice(base.length)}`;
  }
  return path;
}

/**
 * Labels for a set of projects. Projects sharing a name get the fewest trailing parent folders
 * that make every label in that name group distinct: `app · a`, `app · b`.
 */
export function labelProjects(
  projects: ReadonlyArray<{ key: string; name: string }>,
  home?: string
): Map<string, ProjectLabel> {
  const labels = new Map<string, ProjectLabel>();
  const byName = new Map<string, string[]>();
  for (const project of projects) {
    if (labels.has(project.key)) continue;
    labels.set(project.key, {
      name: project.name,
      hint: "",
      path: abbreviateHome(project.key, home),
    });
    const group = byName.get(project.name);
    if (group) group.push(project.key);
    else byName.set(project.name, [project.key]);
  }
  for (const keys of byName.values()) {
    if (keys.length < 2) continue;
    const parents = keys.map((key) => splitPath(key).slice(0, -1).reverse());
    const deepest = Math.max(...parents.map((chain) => chain.length));
    let depth = 1;
    const hintAt = (chain: string[], d: number) => chain.slice(0, d).reverse().join("/");
    while (depth < deepest) {
      const hints = parents.map((chain) => hintAt(chain, depth));
      if (new Set(hints).size === hints.length) break;
      depth += 1;
    }
    keys.forEach((key, index) => {
      const label = labels.get(key);
      if (label) label.hint = hintAt(parents[index], depth);
    });
  }
  return labels;
}

/** Every whitespace-separated token must occur in the name, hint or path (case-insensitive). */
export function matchesQuery(label: ProjectLabel, query: string): boolean {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!tokens.length) return true;
  const haystack = `${label.name}\n${label.hint}\n${label.path}`.toLowerCase();
  return tokens.every((token) => haystack.includes(token));
}

export function buildSidebarModel(
  projects: ReadonlyArray<TodoProjectInfo>,
  query: string,
  home?: string
): SidebarModel {
  const labels = labelProjects(projects, home);
  const withTodos: SidebarRow[] = [];
  const recent: SidebarRow[] = [];
  let allCount = 0;
  for (const project of projects) {
    allCount += project.count;
    const label = labels.get(project.key);
    if (!label || !matchesQuery(label, query)) continue;
    const row: SidebarRow = { ...label, key: project.key, count: project.count };
    (project.section === "withTodos" ? withTodos : recent).push(row);
  }
  const sections: SidebarSection[] = [];
  if (withTodos.length) sections.push({ id: "withTodos", rows: withTodos });
  if (recent.length) sections.push({ id: "recent", rows: recent });
  const rowCount = withTodos.length + recent.length;
  const navKeys = projects.length
    ? [ALL_KEY, ...withTodos.map((row) => row.key), ...recent.map((row) => row.key)]
    : [];
  return { allCount, sections, navKeys, rowCount };
}

/**
 * The key one step away from `current` in keyboard order; clamps at both ends (no wrap-around).
 * An unknown `current` enters the list from the matching end.
 */
export function stepSelection(
  navKeys: ReadonlyArray<string>,
  current: string | null,
  delta: -1 | 1
): string | null {
  if (!navKeys.length) return null;
  const index = current === null ? -1 : navKeys.indexOf(current);
  if (index < 0) return delta > 0 ? navKeys[0] : navKeys[navKeys.length - 1];
  return navKeys[Math.max(0, Math.min(navKeys.length - 1, index + delta))];
}

export interface AllGroup extends ProjectLabel {
  key: string;
  /** Pending todos of the project, in queue order. */
  entries: TodoEntry[];
  /** Number of pending todos (stays the full count after `limitRows` cut the entries). */
  count: number;
}

export interface AllModel {
  groups: AllGroup[];
  projectCount: number;
  total: number;
}

/**
 * Orders the aggregate like the sidebar (projects the sidebar does not know come last, by name),
 * drops empty groups and labels each group like its sidebar row.
 */
export function buildAllModel(
  groups: ReadonlyArray<TodoGroup>,
  projects: ReadonlyArray<TodoProjectInfo>,
  home?: string
): AllModel {
  const known = new Set(projects.map((project) => project.key));
  const strangers = groups
    .filter((group) => !known.has(group.key))
    .map((group) => ({ key: group.key, name: baseName(group.key) }));
  const labels = labelProjects([...projects, ...strangers], home);
  const rank = new Map<string, number>();
  projects.forEach((project, index) => rank.set(project.key, index));

  const out: AllGroup[] = [];
  let total = 0;
  for (const group of groups) {
    if (!group.entries.length) continue;
    const label = labels.get(group.key) ?? {
      name: baseName(group.key),
      hint: "",
      path: group.key,
    };
    out.push({ ...label, key: group.key, entries: group.entries, count: group.entries.length });
    total += group.entries.length;
  }
  const last = projects.length;
  out.sort((a, b) => {
    const ra = rank.get(a.key) ?? last;
    const rb = rank.get(b.key) ?? last;
    if (ra !== rb) return ra - rb;
    return a.name.localeCompare(b.name) || a.key.localeCompare(b.key);
  });
  return { groups: out, projectCount: out.length, total };
}

/** Keeps the first `limit` entries (in display order); groups left without entries are dropped. */
export function limitRows(
  groups: ReadonlyArray<AllGroup>,
  limit: number
): { groups: AllGroup[]; hidden: number } {
  const out: AllGroup[] = [];
  let left = Math.max(0, limit);
  let hidden = 0;
  for (const group of groups) {
    if (left <= 0) {
      hidden += group.entries.length;
      continue;
    }
    if (group.entries.length <= left) {
      out.push(group);
      left -= group.entries.length;
    } else {
      out.push({ ...group, entries: group.entries.slice(0, left) });
      hidden += group.entries.length - left;
      left = 0;
    }
  }
  return { groups: out, hidden };
}
