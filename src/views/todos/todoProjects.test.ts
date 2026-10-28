import { describe, expect, it } from "vitest";
import type { TodoEntry, TodoGroup, TodoProjectInfo } from "../../lib/types";
import {
  ALL_KEY,
  abbreviateHome,
  baseName,
  buildAllModel,
  buildSidebarModel,
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
