import { createI18n } from "vue-i18n";
import { describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import zh from "../../i18n/zh";
import type { DropLogEntry } from "../../lib/types";
import {
  addedCount, createDragLog, describeIgnored, describeResult, emptyResult, failureFrom, fileName, NOTICE_MS, outcomeOf, resultLog,
  type DropFailure, type DropResult, type DropWords, type Translate,
} from "./dropFeedback";

// What to tell the user about a drop, and what to write to the log (docs/specs/popup-inbox-collapse.md §6.9).
const composers = {
  en: createI18n({ legacy: false, locale: "en", messages: { en } }).global,
  zh: createI18n({ legacy: false, locale: "zh", messages: { zh } }).global,
};
function words(lang: "en" | "zh" = "en", mac = false): DropWords {
  const composer = composers[lang];
  const t: Translate = (key, named, plural) => plural === undefined ? composer.t(key, named ?? {}) : composer.t(key, named ?? {}, plural);
  return { t, mac, paste: mac ? "⌘V" : "Ctrl+V" };
}
const failure = (reason: DropFailure["reason"], place: DropFailure["place"] = "other", name = "shot.png", message = "Operation not permitted (os error 1)"): DropFailure =>
  ({ path: `/Users/me/Desktop/${name}`, name, reason, place, message });
const result = (parts: Partial<DropResult> = {}): DropResult => ({ ...emptyResult(parts.requested ?? 1), ...parts });

describe("the files that were dropped", () => {
  it("names a file by what follows the last separator, whichever the platform's", () => {
    expect(fileName("/Users/me/shot.png")).toBe("shot.png");
    expect(fileName("C:\\Users\\me\\shot.png")).toBe("shot.png");
    expect(fileName("shot.png")).toBe("shot.png");
    expect(fileName("/Users/me/")).toBe("file");
    expect(fileName("")).toBe("file");
  });

  it("counts what was attached, whatever it was", () => {
    expect(addedCount(result({ requested: 5, images: 2, files: 1, duplicates: 2 }))).toBe(3);
    expect(addedCount(result())).toBe(0);
  });
});

describe("a read that failed", () => {
  it("is what the backend said: why, and where the file was", () => {
    expect(failureFrom("/Users/me/Desktop/shot.png", { reason: "permission_denied", where: "desktop", message: "Operation not permitted (os error 1)" })).toEqual({
      path: "/Users/me/Desktop/shot.png", name: "shot.png", reason: "permission_denied", place: "desktop", message: "Operation not permitted (os error 1)",
    });
  });

  it("is a plain read error when what came back is not the backend's", () => {
    expect(failureFrom("/tmp/a.png", new Error("unreadable"))).toMatchObject({ reason: "other", place: "other", message: "unreadable" });
    expect(failureFrom("/tmp/a.png", "refused")).toMatchObject({ reason: "other", place: "other", message: "refused" });
    expect(failureFrom("/tmp/a.png", undefined)).toMatchObject({ reason: "other", place: "other", message: "undefined" });
  });

  it("does not trust a reason or a place it does not know", () => {
    expect(failureFrom("/tmp/a.png", { reason: "exploded", where: "the moon", message: "x" })).toMatchObject({ reason: "other", place: "other" });
  });
});

describe("what a drop came to, as the log tells it", () => {
  it("is one word for each way a drop can end", () => {
    expect(outcomeOf(result({ refused: true }))).toBe("not_accepting");
    expect(outcomeOf(result({ requested: 0 }))).toBe("no_paths");
    expect(outcomeOf(result({ files: 1 }))).toBe("added");
    expect(outcomeOf(result({ images: 1 }))).toBe("added");
    expect(outcomeOf(result({ requested: 2, files: 1, failures: [failure("not_found")] }))).toBe("partial");
    expect(outcomeOf(result({ failures: [failure("permission_denied")] }))).toBe("read_failed");
    expect(outcomeOf(result({ duplicates: 1 }))).toBe("duplicates");
  });

  it("is counts, and the failed paths with their reasons, and no name and no system message", () => {
    const failed = failure("permission_denied", "desktop", "secret plan.png", "Operation not permitted (os error 1)");
    expect(resultLog(result({ requested: 3, images: 1, files: 1, duplicates: 0, failures: [failed] }))).toEqual({
      outcome: "partial", added: 2, images: 1, files: 1, duplicates: 0,
      failed: [{ path: "/Users/me/Desktop/secret plan.png", reason: "permission_denied" }],
    });
  });
});

describe("a drop that was turned away", () => {
  it.each([
    ["no_paths", "No files received", "没有收到文件"],
    ["on_preview", "The attachment preview does not take files", "右侧预览区不接收文件"],
    ["busy", "The window is busy", "窗口正忙"],
  ] as const)("is told as a warning: %s", (reason, english, chinese) => {
    expect(describeIgnored(reason, words("en"))).toMatchObject({ level: "warn" });
    expect(describeIgnored(reason, words("en")).text).toContain(english);
    expect(describeIgnored(reason, words("zh")).text).toContain(chinese);
  });

  it("offers the paste shortcut for what is not a file", () => {
    expect(describeIgnored("no_paths", words("en", true)).text).toContain("⌘V");
    expect(describeIgnored("no_paths", words("zh")).text).toContain("Ctrl+V");
  });

  it("names the request that takes no attachments, when the window has several", () => {
    expect(describeIgnored("not_accepting", words("en"), "Deploy?").text).toBe("“Deploy?” does not take attachments");
    expect(describeIgnored("not_accepting", words("zh"), "Deploy?").text).toBe("「Deploy?」不接收附件");
    expect(describeIgnored("not_accepting", words("en")).text).toBe("This question does not take attachments");
  });
});

describe("a drop that was handed to a request", () => {
  const lone = { announce: false };
  const several = { announce: true, title: "Deploy?" };

  it("says it was turned away when the request takes no files", () => {
    expect(describeResult(result({ refused: true }), words(), several)).toEqual({ level: "warn", text: "“Deploy?” does not take attachments" });
    expect(describeResult(result({ refused: true }), words(), lone)).toEqual({ level: "warn", text: "This question does not take attachments" });
  });

  it("says there were no files when it carried none", () => {
    expect(describeResult(result({ requested: 0 }), words(), lone)?.text).toContain("No files received");
  });

  it("says as plain information that everything was attached already", () => {
    expect(describeResult(result({ duplicates: 2, requested: 2 }), words(), lone)).toEqual({ level: "info", text: "Those files are already attached" });
    expect(describeResult(result({ duplicates: 2, requested: 2 }), words("zh"), lone)?.text).toBe("这些文件已在附件里");
  });

  it("says nothing of what was added for a popup that shows it being added", () => {
    expect(describeResult(result({ files: 1 }), words(), lone)).toBeNull();
    expect(describeResult(result({ files: 1 }), words(), { announce: true })).toBeNull();
  });

  it("says what was added, where, when the window has to tell it", () => {
    expect(describeResult(result({ files: 1 }), words(), several)).toEqual({ level: "info", text: "Added 1 file to “Deploy?”" });
    expect(describeResult(result({ requested: 3, files: 2, images: 1 }), words(), several)?.text).toBe("Added 3 files to “Deploy?”");
    expect(describeResult(result({ files: 1 }), words(), { ...several, question: 2 })?.text).toBe("Added 1 file to “Deploy?”, question 3");
    expect(describeResult(result({ requested: 2, files: 2 }), words("zh"), several)?.text).toContain("2");
  });

  describe("when a picture could not be read", () => {
    it("names it and says it was refused, and where to allow it, on a Mac and in a folder the system guards", () => {
      const refused = result({ failures: [failure("permission_denied", "desktop")] });
      const text = describeResult(refused, words("en", true), lone)!.text;
      expect(text).toBe("No permission to read “shot.png”. macOS restricts access to Desktop: open System Settings → Privacy & Security → Files & Folders, "
        + "allow AskHuman (or the terminal or editor that started it) to access Desktop, then drop again");
      expect(describeResult(refused, words("en", true), lone)!.level).toBe("warn");
      expect(describeResult(refused, words("zh", true), lone)!.text).toBe("没有权限读取「shot.png」。macOS 限制了对「桌面」的访问：到 系统设置 → 隐私与安全性 → 文件和文件夹，"
        + "允许 AskHuman（或启动它的终端 / 编辑器）访问「桌面」，再拖一次");
    });

    it.each([
      ["documents", "Documents", "文稿"],
      ["downloads", "Downloads", "下载"],
      ["icloud", "iCloud Drive", "iCloud 云盘"],
    ] as const)("names the folder to allow: %s", (place, english, chinese) => {
      const refused = result({ failures: [failure("permission_denied", place)] });
      expect(describeResult(refused, words("en", true), lone)!.text).toContain(`to access ${english}`);
      expect(describeResult(refused, words("zh", true), lone)!.text).toContain(`访问「${chinese}」`);
    });

    it("points at external and network disks as a whole, since their names are the user's", () => {
      const text = describeResult(result({ failures: [failure("permission_denied", "volume")] }), words("en", true), lone)!.text;
      expect(text).toContain("external and network disks");
      expect(text).toContain("Files & Folders");
    });

    it("points at the file's own permissions when no setting of the system is to blame", () => {
      const elsewhere = result({ failures: [failure("permission_denied", "other")] });
      expect(describeResult(elsewhere, words("en", true), lone)!.text).toBe("No permission to read “shot.png”. Check the file permissions");
      const notMac = result({ failures: [failure("permission_denied", "desktop")] });
      expect(describeResult(notMac, words("en", false), lone)!.text).toBe("No permission to read “shot.png”. Check the file permissions");
      expect(describeResult(notMac, words("zh", false), lone)!.text).toBe("没有权限读取「shot.png」，请检查文件的访问权限");
    });

    it("says a file is gone when every failure was that", () => {
      const gone = result({ requested: 2, failures: [failure("not_found", "downloads", "a.png"), failure("not_found", "downloads", "b.png")] });
      expect(describeResult(gone, words(), lone)!.text).toBe("Could not find these 2 files. They may have been moved or deleted");
      expect(describeResult(result({ failures: [failure("not_found")] }), words(), lone)!.text).toBe("Could not find “shot.png”. It may have been moved or deleted");
    });

    it("gives the system's own words for one picture it could not read for another reason", () => {
      const odd = result({ failures: [failure("other", "other", "odd.png", "Is a directory (os error 21)")] });
      expect(describeResult(odd, words(), lone)!.text).toBe("Could not read “odd.png”: Is a directory (os error 21)");
    });

    it("gives each distinct reason, once, for several pictures that failed in different ways", () => {
      const mixed = result({ requested: 3, failures: [failure("permission_denied", "other", "a.png"), failure("not_found", "other", "b.png"), failure("permission_denied", "other", "c.png")] });
      expect(describeResult(mixed, words(), lone)!.text).toBe("Could not read these 3 files (permission denied, file not found)");
      expect(describeResult(mixed, words("zh"), lone)!.text).toBe("无法读取这 3 个文件（没有权限、找不到文件）");
    });

    it("counts what was added and what was not when only some could be read", () => {
      const some = result({ requested: 3, files: 1, images: 1, failures: [failure("not_found", "downloads", "gone.png")] });
      expect(describeResult(some, words(), several)).toEqual({ level: "warn", text: "Added 2 files, 1 could not be read (file not found)" });
      expect(describeResult(some, words("zh"), several)!.text).toBe("已添加 2 个文件，另有 1 个没能读取（找不到文件）");
      const one = result({ requested: 2, files: 1, failures: [failure("not_found")] });
      expect(describeResult(one, words(), lone)!.text).toBe("Added 1 file, 1 could not be read (file not found)");
    });

    it("adds where to allow it when the files that could not be read were refused by the system", () => {
      const some = result({ requested: 2, files: 1, failures: [failure("permission_denied", "downloads")] });
      expect(describeResult(some, words("en", true), lone)!.text).toBe("Added 1 file, 1 could not be read (permission denied). macOS restricts access to Downloads: "
        + "open System Settings → Privacy & Security → Files & Folders, allow AskHuman (or the terminal or editor that started it) to access Downloads, then drop again");
      expect(describeResult(some, words("en", false), lone)!.text).toBe("Added 1 file, 1 could not be read (permission denied)");
    });
  });
});

describe("how long a notice stays", () => {
  it("is longer for a warning, which has to be read and acted on", () => {
    expect(NOTICE_MS.info).toBe(2600);
    expect(NOTICE_MS.warn).toBeGreaterThan(NOTICE_MS.info);
  });
});

describe("the log of a drag", () => {
  function log(base: Partial<DropLogEntry> = { requestId: "r1" }) {
    const sent: DropLogEntry[] = [];
    return { sent, drag: createDragLog(entry => { sent.push(entry); }, () => base) };
  }

  it("writes a drag once, as it starts, however many events announce it", () => {
    const { sent, drag } = log();
    drag.seen("enter", ["/tmp/a", "/tmp/b"]);
    drag.seen("over", undefined);
    drag.seen("over", undefined);
    expect(sent).toEqual([{ requestId: "r1", action: "enter", from: "enter", paths: ["/tmp/a", "/tmp/b"] }]);
  });

  it("writes the first move of a drag whose enter was missed, as such", () => {
    const { sent, drag } = log();
    drag.seen("over", undefined);
    drag.seen("over", undefined);
    expect(sent).toEqual([{ requestId: "r1", action: "enter", from: "over", paths: [] }]);
  });

  it("starts a new drag at every enter", () => {
    const { sent, drag } = log();
    drag.seen("enter", ["/tmp/a"]);
    drag.seen("enter", ["/tmp/b"]);
    expect(sent.map(entry => entry.paths)).toEqual([["/tmp/a"], ["/tmp/b"]]);
  });

  it("carries what the caller adds to the line that starts a drag", () => {
    const { sent, drag } = log();
    drag.seen("enter", [], { outcome: "busy" });
    expect(sent).toEqual([{ requestId: "r1", action: "enter", from: "enter", paths: [], outcome: "busy" }]);
  });

  it("writes a leave only for a drag that started, and only once", () => {
    const { sent, drag } = log();
    drag.left();
    drag.seen("enter", ["/tmp/a"]);
    drag.left();
    drag.left();
    expect(sent.map(entry => entry.action)).toEqual(["enter", "leave"]);
  });

  it("forgets a drag that ended with a drop, so the next one that begins with a move is written", () => {
    const { sent, drag } = log();
    drag.seen("enter", ["/tmp/a"]);
    drag.end();
    drag.left();
    drag.seen("over", undefined);
    expect(sent.map(entry => `${entry.action}:${entry.from ?? ""}`)).toEqual(["enter:enter", "enter:over"]);
  });

  it("reads what every line carries at the moment of the line", () => {
    let view: DropLogEntry["view"] = "collapsed";
    const sent: DropLogEntry[] = [];
    const drag = createDragLog(entry => { sent.push(entry); }, () => ({ view }));
    drag.seen("enter", []);
    view = "expanded";
    drag.send({ action: "drop", outcome: "added" });
    expect(sent.map(entry => entry.view)).toEqual(["collapsed", "expanded"]);
  });

  it("lets a line say better than the base what it is about", () => {
    const { sent, drag } = log({ requestId: "active", view: "expanded" });
    drag.send({ action: "drop", requestId: "dropped-on", outcome: "added" });
    expect(sent).toEqual([{ requestId: "dropped-on", view: "expanded", action: "drop", outcome: "added" }]);
  });

  it("is not the one to fail when sending does not: that is the sender's to handle", () => {
    const send = vi.fn();
    const drag = createDragLog(send, () => ({}));
    drag.seen("enter", []);
    expect(send).toHaveBeenCalledOnce();
  });
});

describe("the words of the notices", () => {
  type Tree = { [key: string]: string | Tree };
  const flatten = (tree: Tree, prefix = ""): Record<string, string> => Object.entries(tree).reduce<Record<string, string>>((all, [key, value]) =>
    typeof value === "string" ? { ...all, [`${prefix}${key}`]: value } : { ...all, ...flatten(value, `${prefix}${key}.`) }, {});
  const english = flatten((en as unknown as { popup: { drop: Tree } }).popup.drop);
  const chinese = flatten((zh as unknown as { popup: { drop: Tree } }).popup.drop);
  const names = (message: string) => [...new Set([...message.matchAll(/\{(\w+)\}/g)].map(match => match[1]))].sort();

  it("exist in both languages, under the same keys", () => {
    expect(Object.keys(chinese).sort()).toEqual(Object.keys(english).sort());
    expect(Object.keys(english).length).toBeGreaterThan(20);
  });

  it("take the same values in both languages", () => {
    for (const key of Object.keys(english)) expect(names(chinese[key]), key).toEqual(names(english[key]));
  });

  it("keep out of the way of the message syntax: no linked messages, and a plural bar only where English counts", () => {
    for (const [key, message] of Object.entries(english)) {
      expect(message, key).not.toContain("@");
      if (!/^(partial|partialFix|notFound)$/.test(key)) expect(message, key).not.toContain("|");
    }
    for (const [key, message] of Object.entries(chinese)) {
      expect(message, key).not.toContain("@");
      expect(message, key).not.toContain("|");
    }
  });

  it("give every key that describeResult and describeIgnored ask for", () => {
    const asked = [
      "noFiles", "onPreview", "notAccepting", "notAcceptingHere", "busy", "duplicates", "whatOne", "whatMany", "joiner",
      "why.permission_denied", "why.not_found", "why.other", "place.desktop", "place.documents", "place.downloads", "place.icloud",
      "fixFolder", "fixVolume", "permission", "permissionPlain", "notFound", "readFailed", "readFailedWhy", "partial", "partialFix",
    ];
    for (const key of asked) {
      expect(english[key], `en ${key}`).toBeTruthy();
      expect(chinese[key], `zh ${key}`).toBeTruthy();
    }
  });
});
