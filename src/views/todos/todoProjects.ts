// Sidebar view model of the todos window (spec todo-sync §9): labels, grouping, search, keyboard
// stepping and the "All todos" aggregate. Pure functions only, so the window stays cheap to open
// (everything here is O(projects) string work) and is unit-testable without a DOM.

import type {
  TodoEntry,
  TodoGroup,
  TodoProjectInfo,
  TodoSyncProject,
} from "../../lib/types";

/** Key of the "All todos" sidebar row. Project keys are absolute paths, so this cannot collide. */
export const ALL_KEY = "__all__";

/**
 * Keys of synced projects this Mac has no directory for start with this (mirror of Rust
 * `todo_sync::cloudkeys::PREFIX`). Their todos live in `todos.json` like any project's.
 */
export const CLOUD_KEY_PREFIX = "icloud:";

export function isCloudKey(key: string): boolean {
  return key.length > CLOUD_KEY_PREFIX.length && key.startsWith(CLOUD_KEY_PREFIX);
}

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

/**
 * Where a project lives, in iCloud mode: `synced` has a directory here, `cloudOnly` exists only
 * in iCloud (this Mac has no directory of the repository), `localOnly` has no git remote.
 */
export type ProjectScope = "synced" | "cloudOnly" | "localOnly";

export interface SidebarRow extends ProjectLabel {
  /** The key a click opens: the project's most recently used directory (or its replica key). */
  key: string;
  count: number;
  /** iCloud mode only. */
  scope?: ProjectScope;
}

/** `withTodos` / `recent` are the local mode's sections, the others are iCloud mode's. */
export type SidebarSectionId = "withTodos" | "recent" | "synced" | "empty" | "localOnly";

export interface SidebarSection {
  id: SidebarSectionId;
  /** Rows shown (none while the `empty` section is folded). */
  rows: SidebarRow[];
  /** Projects in the section, shown or not (the folded section's "(N)"). */
  total: number;
  /** Keys of every project in the section, shown or not. */
  keys: string[];
  /** Only the `empty` section folds. */
  collapsed: boolean;
}

export interface SidebarModel {
  /** Pending todos over all projects (not narrowed by the search). */
  allCount: number;
  /** Non-empty sections only, in display order. */
  sections: SidebarSection[];
  /** Keyboard order: the "All todos" row followed by every visible project row. */
  navKeys: string[];
  /** Number of project rows after the search filter (folded ones included). */
  rowCount: number;
  /** Label of every selectable key, whether or not the search shows it. */
  labelOf: ReadonlyMap<string, ProjectLabel>;
  /** The row a key belongs to: the directories of one repository share a row (iCloud mode). */
  rowKeyOf: ReadonlyMap<string, string>;
}

export interface SidebarOptions {
  /** `todo_sync_projects`; empty or absent = local mode. */
  sync?: ReadonlyArray<TodoSyncProject> | null;
  /** The "projects without todos" section is unfolded. */
  emptyOpen?: boolean;
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

/**
 * Every whitespace-separated token must occur in the name, hint or path (case-insensitive), or in
 * `extra` (a synced project's directories, which the row itself does not show).
 */
export function matchesQuery(label: ProjectLabel, query: string, extra = ""): boolean {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!tokens.length) return true;
  const haystack = `${label.name}\n${label.hint}\n${label.path}\n${extra}`.toLowerCase();
  return tokens.every((token) => haystack.includes(token));
}

/** A project as iCloud mode sees it: directories of one repository merged into one. */
interface MergedProject {
  /** What a click opens. */
  key: string;
  /** Every key that belongs to the project (directories and the replica key). */
  keys: string[];
  count: number;
  scope: ProjectScope;
  /** Normalized remote; `null` for projects without one. */
  identity: string | null;
  name: string;
}

/**
 * Folds `projects` (in the backend's order, which ranks the most recently used directory first)
 * into one entry per repository, then appends synced projects the list does not mention (a
 * project whose todos are all finished lives only in the history).
 */
function mergeProjects(
  projects: ReadonlyArray<TodoProjectInfo>,
  sync: ReadonlyArray<TodoSyncProject>
): MergedProject[] {
  const bySyncKey = new Map<string, TodoSyncProject>();
  for (const project of sync) {
    for (const key of project.keys) bySyncKey.set(key, project);
    if (project.cloudKey) bySyncKey.set(project.cloudKey, project);
  }
  const merged: MergedProject[] = [];
  const byIdentity = new Map<string, MergedProject>();
  const fromSync = (key: string, count: number, project: TodoSyncProject): MergedProject => ({
    key,
    keys: project.cloudKey ? [...project.keys, project.cloudKey] : [...project.keys],
    count,
    scope: project.keys.length ? "synced" : "cloudOnly",
    identity: project.identity,
    name: baseName(project.identity),
  });
  for (const info of projects) {
    const project = bySyncKey.get(info.key);
    if (!project) {
      merged.push({
        key: info.key,
        keys: [info.key],
        count: info.count,
        scope: "localOnly",
        identity: null,
        name: info.name,
      });
      continue;
    }
    const seen = byIdentity.get(project.identity);
    if (seen) {
      // Replicas agree once the local phase ran; until then the larger count is the newer one.
      seen.count = Math.max(seen.count, info.count);
      continue;
    }
    const entry = fromSync(info.key, info.count, project);
    byIdentity.set(project.identity, entry);
    merged.push(entry);
  }
  for (const project of sync) {
    if (byIdentity.has(project.identity)) continue;
    const key = project.keys[0] ?? project.cloudKey;
    if (!key) continue;
    const entry = fromSync(key, 0, project);
    byIdentity.set(project.identity, entry);
    merged.push(entry);
  }
  return merged;
}

/**
 * Labels of merged projects: synced ones by repository address (`github.com/acme/app` → `app`,
 * same-named repositories told apart by owner), the others by directory like the local mode.
 */
function labelMerged(
  merged: ReadonlyArray<MergedProject>,
  home?: string
): Map<MergedProject, ProjectLabel> {
  const remote = labelProjects(
    merged.filter((m) => m.identity !== null).map((m) => ({ key: m.identity as string, name: m.name })),
    home
  );
  const local = labelProjects(
    merged.filter((m) => m.identity === null).map((m) => ({ key: m.key, name: m.name })),
    home
  );
  const out = new Map<MergedProject, ProjectLabel>();
  for (const m of merged) {
    const label = (m.identity === null ? local.get(m.key) : remote.get(m.identity)) ?? {
      name: m.name,
      hint: "",
      path: m.key,
    };
    out.set(m, label);
  }
  return out;
}

/** Directories of a merged project as the search sees them. */
function directoriesText(m: MergedProject, home?: string): string {
  return m.keys
    .filter((key) => !isCloudKey(key))
    .map((key) => abbreviateHome(key, home))
    .join("\n");
}

function section(id: SidebarSectionId, rows: SidebarRow[]): SidebarSection {
  return { id, rows, total: rows.length, keys: rows.map((row) => row.key), collapsed: false };
}

export function buildSidebarModel(
  projects: ReadonlyArray<TodoProjectInfo>,
  query: string,
  home?: string,
  options: SidebarOptions = {}
): SidebarModel {
  if (options.sync?.length) return buildSyncedSidebarModel(projects, query, home, options);
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
  if (withTodos.length) {
    sections.push(section("withTodos", withTodos));
  }
  if (recent.length) {
    sections.push(section("recent", recent));
  }
  const rowCount = withTodos.length + recent.length;
  const navKeys = projects.length
    ? [ALL_KEY, ...withTodos.map((row) => row.key), ...recent.map((row) => row.key)]
    : [];
  return { allCount, sections, navKeys, rowCount, labelOf: labels, rowKeyOf: new Map() };
}

/**
 * iCloud mode: one row per repository, "iCloud projects" first, the projects without todos folded
 * into one section, projects without a remote last (spec todo-sync §9.2).
 */
function buildSyncedSidebarModel(
  projects: ReadonlyArray<TodoProjectInfo>,
  query: string,
  home: string | undefined,
  options: SidebarOptions
): SidebarModel {
  const merged = mergeProjects(projects, options.sync ?? []);
  const labels = labelMerged(merged, home);
  const synced: SidebarRow[] = [];
  const empty: SidebarRow[] = [];
  const localOnly: SidebarRow[] = [];
  const labelOf = new Map<string, ProjectLabel>();
  const rowKeyOf = new Map<string, string>();
  let allCount = 0;
  for (const m of merged) {
    allCount += m.count;
    const label = labels.get(m) as ProjectLabel;
    for (const key of m.keys) {
      labelOf.set(key, label);
      rowKeyOf.set(key, m.key);
    }
    if (!matchesQuery(label, query, directoriesText(m, home))) continue;
    const row: SidebarRow = { ...label, key: m.key, count: m.count, scope: m.scope };
    if (m.count === 0) empty.push(row);
    else if (m.scope === "localOnly") localOnly.push(row);
    else synced.push(row);
  }
  const unfolded = Boolean(options.emptyOpen) || query.trim() !== "";
  const sections: SidebarSection[] = [];
  if (synced.length) {
    sections.push(section("synced", synced));
  }
  if (empty.length) {
    sections.push({ ...section("empty", empty), rows: unfolded ? empty : [], collapsed: !unfolded });
  }
  if (localOnly.length) {
    sections.push(section("localOnly", localOnly));
  }
  const navKeys = merged.length
    ? [
        ALL_KEY,
        ...synced.map((row) => row.key),
        ...(unfolded ? empty.map((row) => row.key) : []),
        ...localOnly.map((row) => row.key),
      ]
    : [];
  return {
    allCount,
    sections,
    navKeys,
    rowCount: synced.length + empty.length + localOnly.length,
    labelOf,
    rowKeyOf,
  };
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
  /** iCloud mode only. */
  scope?: ProjectScope;
}

export interface AllModel {
  groups: AllGroup[];
  projectCount: number;
  total: number;
  /** Projects without a remote (iCloud mode only; 0 otherwise). */
  localOnlyCount: number;
}

/**
 * Orders the aggregate like the sidebar (projects the sidebar does not know come last, by name),
 * drops empty groups and labels each group like its sidebar row.
 */
export function buildAllModel(
  groups: ReadonlyArray<TodoGroup>,
  projects: ReadonlyArray<TodoProjectInfo>,
  home?: string,
  sync?: ReadonlyArray<TodoSyncProject> | null
): AllModel {
  if (sync?.length) return buildSyncedAllModel(groups, projects, home, sync);
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
  return { groups: out, projectCount: out.length, total, localOnlyCount: 0 };
}

/**
 * iCloud mode: the replicas of one repository are one group (the one under the project's own row
 * key, else any that has entries); groups the project list does not know come last, as local-only.
 */
function buildSyncedAllModel(
  groups: ReadonlyArray<TodoGroup>,
  projects: ReadonlyArray<TodoProjectInfo>,
  home: string | undefined,
  sync: ReadonlyArray<TodoSyncProject>
): AllModel {
  const merged = mergeProjects(projects, sync);
  const labels = labelMerged(merged, home);
  const byKey = new Map<string, TodoGroup>();
  for (const group of groups) byKey.set(group.key, group);

  const out: AllGroup[] = [];
  const used = new Set<string>();
  let total = 0;
  let localOnlyCount = 0;
  for (const m of merged) {
    const own = byKey.get(m.key);
    const group = own?.entries.length
      ? own
      : m.keys.map((key) => byKey.get(key)).find((g) => g?.entries.length);
    for (const key of m.keys) used.add(key);
    used.add(m.key);
    if (!group?.entries.length) continue;
    const label = labels.get(m) as ProjectLabel;
    out.push({ ...label, key: group.key, entries: group.entries, count: group.entries.length, scope: m.scope });
    total += group.entries.length;
    if (m.scope === "localOnly") localOnlyCount += 1;
  }
  // Like the sidebar: projects without a remote come after the synced ones (stable, so each part
  // keeps the backend's ranking).
  out.sort((a, b) => Number(a.scope === "localOnly") - Number(b.scope === "localOnly"));
  const strangers = groups
    .filter((group) => group.entries.length && !used.has(group.key))
    .map((group) => ({ key: group.key, name: baseName(group.key) }));
  const strangerLabels = labelProjects(strangers, home);
  strangers.sort((a, b) => a.name.localeCompare(b.name) || a.key.localeCompare(b.key));
  for (const stranger of strangers) {
    const group = byKey.get(stranger.key) as TodoGroup;
    const label = strangerLabels.get(stranger.key) as ProjectLabel;
    out.push({
      ...label,
      key: group.key,
      entries: group.entries,
      count: group.entries.length,
      scope: "localOnly",
    });
    total += group.entries.length;
    localOnlyCount += 1;
  }
  return { groups: out, projectCount: out.length, total, localOnlyCount };
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
