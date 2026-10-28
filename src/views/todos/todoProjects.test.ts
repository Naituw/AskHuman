import { describe, expect, it } from "vitest";
import type { TodoEntry, TodoGroup, TodoProjectInfo, TodoSyncProject } from "../../lib/types";
import {
  ALL_KEY,
  abbreviateHome,
  baseName,
  buildAllModel,
  buildSidebarModel,
  isCloudKey,
  labelProjects,
  limitRows,
  matchesQuery,
  stepSelection,
} from "./todoProjects";

function project(
  key: string,
  count = 0,
  section: TodoProjectInfo["section"] = count ? "withTodos" : "recent"
): TodoProjectInfo {
  return { key, name: baseName(key), count, section };
}

function entry(id: string): TodoEntry {
  return { id, text: `todo ${id}`, createdAtMs: 1 };
}

function group(key: string, ...ids: string[]): TodoGroup {
  return { key, entries: ids.map(entry) };
}

describe("abbreviateHome", () => {
  it("shows the home directory as ~", () => {
    expect(abbreviateHome("/Users/me/dev/app", "/Users/me")).toBe("~/dev/app");
    expect(abbreviateHome("/Users/me", "/Users/me")).toBe("~");
    expect(abbreviateHome("/Users/me/", "/Users/me/")).toBe("~/");
  });

  it("leaves other paths and unknown homes alone", () => {
    expect(abbreviateHome("/Users/mellow/app", "/Users/me")).toBe("/Users/mellow/app");
    expect(abbreviateHome("/srv/app", "/Users/me")).toBe("/srv/app");
    expect(abbreviateHome("/Users/me/app", undefined)).toBe("/Users/me/app");
    expect(abbreviateHome("/Users/me/app", "")).toBe("/Users/me/app");
  });

  it("understands Windows separators", () => {
    expect(abbreviateHome("C:\\Users\\me\\dev", "C:\\Users\\me")).toBe("~\\dev");
  });
});

describe("labelProjects", () => {
  it("leaves unique names without a hint", () => {
    const labels = labelProjects([project("/a/app"), project("/a/blog")], "/a");
    expect(labels.get("/a/app")).toEqual({ name: "app", hint: "", path: "~/app" });
    expect(labels.get("/a/blog")?.hint).toBe("");
  });

  it("tells same-named projects apart by their parent folder", () => {
    const labels = labelProjects([project("/x/a/app"), project("/x/b/app"), project("/x/blog")]);
    expect(labels.get("/x/a/app")?.hint).toBe("a");
    expect(labels.get("/x/b/app")?.hint).toBe("b");
    expect(labels.get("/x/blog")?.hint).toBe("");
  });

  it("goes up as far as needed to make the hints distinct", () => {
    const labels = labelProjects([project("/x/a/app"), project("/y/a/app"), project("/z/b/app")]);
    expect(labels.get("/x/a/app")?.hint).toBe("x/a");
    expect(labels.get("/y/a/app")?.hint).toBe("y/a");
    expect(labels.get("/z/b/app")?.hint).toBe("z/b");
  });

  it("copes with a path that has fewer parents than its twin", () => {
    const labels = labelProjects([project("/app"), project("/a/app")]);
    expect(labels.get("/app")?.hint).toBe("");
    expect(labels.get("/a/app")?.hint).toBe("a");
  });
});

describe("matchesQuery", () => {
  const label = { name: "HumanInLoop", hint: "feature-a", path: "~/Developer/HumanInLoop" };

  it("matches every token anywhere in name, hint or path, ignoring case", () => {
    expect(matchesQuery(label, "")).toBe(true);
    expect(matchesQuery(label, "  ")).toBe(true);
    expect(matchesQuery(label, "human")).toBe(true);
    expect(matchesQuery(label, "FEATURE")).toBe(true);
    expect(matchesQuery(label, "developer loop")).toBe(true);
    expect(matchesQuery(label, "human nope")).toBe(false);
  });
});

describe("buildSidebarModel", () => {
  const projects = [
    project("/w/a", 4),
    project("/w/b", 2),
    project("/w/c"),
    project("/w/d"),
  ];

  it("splits the sections, keeps the server order and counts every project", () => {
    const model = buildSidebarModel(projects, "");
    expect(model.allCount).toBe(6);
    expect(model.sections.map((s) => s.id)).toEqual(["withTodos", "recent"]);
    expect(model.sections[0].rows.map((r) => r.key)).toEqual(["/w/a", "/w/b"]);
    expect(model.sections[1].rows.map((r) => r.key)).toEqual(["/w/c", "/w/d"]);
    expect(model.navKeys).toEqual([ALL_KEY, "/w/a", "/w/b", "/w/c", "/w/d"]);
    expect(model.rowCount).toBe(4);
  });

  it("treats unknown sections as recent", () => {
    const model = buildSidebarModel([project("/w/x", 0, "somethingNew")], "");
    expect(model.sections.map((s) => s.id)).toEqual(["recent"]);
  });

  it("narrows rows by the search but not the total", () => {
    const model = buildSidebarModel(projects, "b");
    expect(model.allCount).toBe(6);
    expect(model.sections).toHaveLength(1);
    expect(model.sections[0].rows.map((r) => r.key)).toEqual(["/w/b"]);
    expect(model.navKeys).toEqual([ALL_KEY, "/w/b"]);
  });

  it("drops empty sections and has no rows for an unmatched search", () => {
    const model = buildSidebarModel(projects, "zzz");
    expect(model.sections).toEqual([]);
    expect(model.rowCount).toBe(0);
    expect(model.navKeys).toEqual([ALL_KEY]);
  });

  it("has nothing to navigate without projects", () => {
    expect(buildSidebarModel([], "").navKeys).toEqual([]);
  });

  it("carries the ~ path and the disambiguation hint on rows", () => {
    const model = buildSidebarModel(
      [project("/Users/me/a/app", 1), project("/Users/me/b/app", 1)],
      "",
      "/Users/me"
    );
    const rows = model.sections[0].rows;
    expect(rows.map((r) => r.hint)).toEqual(["a", "b"]);
    expect(rows[0].path).toBe("~/a/app");
  });
});

describe("stepSelection", () => {
  const keys = [ALL_KEY, "/a", "/b"];

  it("moves one step and clamps at the ends", () => {
    expect(stepSelection(keys, ALL_KEY, 1)).toBe("/a");
    expect(stepSelection(keys, "/a", 1)).toBe("/b");
    expect(stepSelection(keys, "/b", 1)).toBe("/b");
    expect(stepSelection(keys, "/a", -1)).toBe(ALL_KEY);
    expect(stepSelection(keys, ALL_KEY, -1)).toBe(ALL_KEY);
  });

  it("enters from the matching end when the current key is not listed", () => {
    expect(stepSelection(keys, "/gone", 1)).toBe(ALL_KEY);
    expect(stepSelection(keys, "/gone", -1)).toBe("/b");
    expect(stepSelection(keys, null, 1)).toBe(ALL_KEY);
  });

  it("returns null for an empty list", () => {
    expect(stepSelection([], null, 1)).toBeNull();
  });
});

describe("buildAllModel", () => {
  const projects = [project("/w/a", 2), project("/w/b", 1), project("/w/c")];

  it("orders groups like the sidebar and totals the todos", () => {
    const model = buildAllModel(
      [group("/w/b", "b1"), group("/w/a", "a1", "a2"), group("/w/c")],
      projects
    );
    expect(model.groups.map((g) => g.key)).toEqual(["/w/a", "/w/b"]);
    expect(model.groups[0].count).toBe(2);
    expect(model.projectCount).toBe(2);
    expect(model.total).toBe(3);
  });

  it("puts projects the sidebar does not know last, by name", () => {
    const model = buildAllModel(
      [group("/z/zed", "z1"), group("/w/a", "a1"), group("/y/ant", "n1")],
      projects
    );
    expect(model.groups.map((g) => g.key)).toEqual(["/w/a", "/y/ant", "/z/zed"]);
    expect(model.groups[1].name).toBe("ant");
  });

  it("labels groups like their sidebar rows", () => {
    const model = buildAllModel(
      [group("/Users/me/a/app", "1"), group("/Users/me/b/app", "2")],
      [project("/Users/me/a/app", 1), project("/Users/me/b/app", 1)],
      "/Users/me"
    );
    expect(model.groups.map((g) => g.hint)).toEqual(["a", "b"]);
    expect(model.groups[0].path).toBe("~/a/app");
  });
});

describe("limitRows", () => {
  const model = buildAllModel(
    [group("/w/a", "a1", "a2", "a3"), group("/w/b", "b1", "b2")],
    [project("/w/a", 3), project("/w/b", 2)]
  );

  it("keeps everything when it fits", () => {
    const cut = limitRows(model.groups, 5);
    expect(cut.hidden).toBe(0);
    expect(cut.groups).toHaveLength(2);
  });

  it("cuts inside a group and reports the rest, keeping the full count", () => {
    const cut = limitRows(model.groups, 2);
    expect(cut.hidden).toBe(3);
    expect(cut.groups).toHaveLength(1);
    expect(cut.groups[0].entries.map((e) => e.id)).toEqual(["a1", "a2"]);
    expect(cut.groups[0].count).toBe(3);
  });

  it("drops groups that no longer have a visible row", () => {
    const cut = limitRows(model.groups, 3);
    expect(cut.groups.map((g) => g.key)).toEqual(["/w/a"]);
    expect(cut.hidden).toBe(2);
  });

  it("shows nothing for a zero limit", () => {
    const cut = limitRows(model.groups, 0);
    expect(cut.groups).toEqual([]);
    expect(cut.hidden).toBe(5);
  });
});

// ---- iCloud mode (spec todo-sync §9.2–§9.4) ----------------------------------------------------

function synced(identity: string, keys: string[], cloudKey: string | null = null): TodoSyncProject {
  return {
    identity,
    keys,
    cloudKey,
    macs: keys.length ? [{ name: "Studio", lastSyncMs: 1, thisMac: true }] : [],
  };
}

const HOME = "/Users/me";
const APP = "github.com/acme/app";
const DOTS = "github.com/acme/dotfiles";
const CLOUD_DOTS = "icloud:github.com/acme/dotfiles";

/** Two directories of one repository, a cloud-only project, an empty one and a project without remote. */
function iCloudWorld() {
  const projects = [
    project("/Users/me/dev/app-wt", 3),
    project("/Users/me/dev/app", 3),
    project(CLOUD_DOTS, 2),
    project("/Users/me/Scratch/notes", 2),
    project("/Users/me/dev/blog", 0),
    project("/Users/me/dev/lonely", 0),
  ];
  const sync = [
    synced(APP, ["/Users/me/dev/app", "/Users/me/dev/app-wt"]),
    synced(DOTS, [], CLOUD_DOTS),
    synced("github.com/acme/blog", ["/Users/me/dev/blog"]),
  ];
  return { projects, sync };
}

describe("isCloudKey", () => {
  it("recognizes the replica keys of projects without a directory", () => {
    expect(isCloudKey(CLOUD_DOTS)).toBe(true);
    expect(isCloudKey("icloud:")).toBe(false);
    expect(isCloudKey("/Users/me/icloud:dev")).toBe(false);
  });
});

describe("buildSidebarModel in iCloud mode", () => {
  it("merges directories of one repository into the row of the most recently used one", () => {
    const { projects, sync } = iCloudWorld();
    const model = buildSidebarModel(projects, "", HOME, { sync });
    const syncedRows = model.sections.find((s) => s.id === "synced")?.rows ?? [];
    expect(syncedRows.map((r) => [r.name, r.key, r.count, r.scope])).toEqual([
      ["app", "/Users/me/dev/app-wt", 3, "synced"],
      ["dotfiles", CLOUD_DOTS, 2, "cloudOnly"],
    ]);
    expect(syncedRows[0].path).toBe(APP);
    // The merged row stands for every directory of the project.
    expect(model.rowKeyOf.get("/Users/me/dev/app")).toBe("/Users/me/dev/app-wt");
    expect(model.rowKeyOf.get(CLOUD_DOTS)).toBe(CLOUD_DOTS);
    expect(model.labelOf.get("/Users/me/dev/app")?.name).toBe("app");
  });

  it("counts each project once, however many directories it has", () => {
    const { projects, sync } = iCloudWorld();
    expect(buildSidebarModel(projects, "", HOME, { sync }).allCount).toBe(3 + 2 + 2);
  });

  it("folds the projects without todos and counts them, unfolding on request or search", () => {
    const { projects, sync } = iCloudWorld();
    const folded = buildSidebarModel(projects, "", HOME, { sync });
    const empty = folded.sections.find((s) => s.id === "empty");
    expect(empty?.collapsed).toBe(true);
    expect(empty?.total).toBe(2);
    expect(empty?.rows).toEqual([]);
    expect(empty?.keys).toEqual(["/Users/me/dev/blog", "/Users/me/dev/lonely"]);
    expect(folded.navKeys).not.toContain("/Users/me/dev/blog");

    const open = buildSidebarModel(projects, "", HOME, { sync, emptyOpen: true });
    const unfolded = open.sections.find((s) => s.id === "empty");
    expect(unfolded?.collapsed).toBe(false);
    expect(unfolded?.rows.map((r) => r.name)).toEqual(["blog", "lonely"]);
    expect(open.navKeys).toContain("/Users/me/dev/blog");

    // A search looks into the folded rows too.
    const found = buildSidebarModel(projects, "blog", HOME, { sync });
    expect(found.sections.map((s) => s.id)).toEqual(["empty"]);
    expect(found.sections[0].rows.map((r) => r.name)).toEqual(["blog"]);
    expect(found.navKeys).toEqual([ALL_KEY, "/Users/me/dev/blog"]);
  });

  it("lists projects without a remote last, labelled by directory", () => {
    const { projects, sync } = iCloudWorld();
    const model = buildSidebarModel(projects, "", HOME, { sync });
    expect(model.sections.map((s) => s.id)).toEqual(["synced", "empty", "localOnly"]);
    const local = model.sections[2].rows;
    expect(local.map((r) => [r.name, r.path, r.scope])).toEqual([
      ["notes", "~/Scratch/notes", "localOnly"],
    ]);
  });

  it("finds a project by the directories it has on this Mac", () => {
    const { projects, sync } = iCloudWorld();
    const model = buildSidebarModel(projects, "app-wt", HOME, { sync });
    expect(model.sections[0].rows.map((r) => r.name)).toEqual(["app"]);
    // …and by its repository address.
    expect(buildSidebarModel(projects, "acme/dot", HOME, { sync }).rowCount).toBe(1);
  });

  it("tells same-named repositories apart by owner", () => {
    const projects = [project("/x/app", 1), project("/y/app", 2)];
    const sync = [synced("github.com/acme/app", ["/x/app"]), synced("github.com/other/app", ["/y/app"])];
    const rows = buildSidebarModel(projects, "", undefined, { sync }).sections[0].rows;
    expect(rows.map((r) => r.hint)).toEqual(["acme", "other"]);
  });

  it("shows synced projects the list does not mention (all finished: only history)", () => {
    const projects = [project("/Users/me/dev/app", 1)];
    const sync = [synced(APP, ["/Users/me/dev/app"]), synced(DOTS, [], CLOUD_DOTS)];
    const model = buildSidebarModel(projects, "", HOME, { sync, emptyOpen: true });
    const empty = model.sections.find((s) => s.id === "empty");
    expect(empty?.rows.map((r) => [r.key, r.scope, r.count])).toEqual([[CLOUD_DOTS, "cloudOnly", 0]]);
  });

  it("is the local mode when there is nothing synced", () => {
    const { projects } = iCloudWorld();
    const local = buildSidebarModel(projects, "", HOME);
    expect(buildSidebarModel(projects, "", HOME, { sync: [] })).toEqual(local);
    expect(local.sections.map((s) => s.id)).toEqual(["withTodos", "recent"]);
    expect(local.sections.every((s) => !s.collapsed && s.total === s.rows.length)).toBe(true);
  });
});

describe("buildAllModel in iCloud mode", () => {
  it("shows one group per project and marks cloud-only and local-only ones", () => {
    const { projects, sync } = iCloudWorld();
    const groups = [
      group("/Users/me/dev/app", "a1", "a2", "a3"),
      group("/Users/me/dev/app-wt", "a1", "a2", "a3"),
      group(CLOUD_DOTS, "d1", "d2"),
      group("/Users/me/Scratch/notes", "n1", "n2"),
    ];
    const model = buildAllModel(groups, projects, HOME, sync);
    expect(model.groups.map((g) => [g.name, g.scope, g.count])).toEqual([
      ["app", "synced", 3],
      ["dotfiles", "cloudOnly", 2],
      ["notes", "localOnly", 2],
    ]);
    expect(model.total).toBe(7);
    expect(model.projectCount).toBe(3);
    expect(model.localOnlyCount).toBe(1);
    // The group opens the directory that has the entries, preferring the row's own.
    expect(model.groups[0].key).toBe("/Users/me/dev/app-wt");
  });

  it("lists projects without a remote after the synced ones, like the sidebar", () => {
    const projects = [project("/n/notes", 5), project("/n/app", 3), project(CLOUD_DOTS, 2)];
    const sync = [synced(APP, ["/n/app"]), synced(DOTS, [], CLOUD_DOTS)];
    const groups = [group("/n/notes", "n1"), group("/n/app", "a1"), group(CLOUD_DOTS, "d1")];
    const model = buildAllModel(groups, projects, undefined, sync);
    expect(model.groups.map((g) => g.name)).toEqual(["app", "dotfiles", "notes"]);
  });

  it("falls back to another replica when the row's own key has no group", () => {
    const { projects, sync } = iCloudWorld();
    const model = buildAllModel([group("/Users/me/dev/app", "a1")], projects, HOME, sync);
    expect(model.groups.map((g) => g.key)).toEqual(["/Users/me/dev/app"]);
  });

  it("puts groups of unknown projects last as local-only", () => {
    const { projects, sync } = iCloudWorld();
    const model = buildAllModel([group("/elsewhere/zeta", "z1"), group(CLOUD_DOTS, "d1")], projects, HOME, sync);
    expect(model.groups.map((g) => [g.name, g.scope])).toEqual([
      ["dotfiles", "cloudOnly"],
      ["zeta", "localOnly"],
    ]);
    expect(model.localOnlyCount).toBe(1);
  });

  it("is the local aggregate when nothing is synced", () => {
    const { projects } = iCloudWorld();
    const groups = [group("/Users/me/dev/app", "a1")];
    expect(buildAllModel(groups, projects, HOME, [])).toEqual(buildAllModel(groups, projects, HOME));
    expect(buildAllModel(groups, projects, HOME).groups[0].scope).toBeUndefined();
  });
});
