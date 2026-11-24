import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { test } from "node:test";

const installedApp = ["/Applications/Codex.app", "/Applications/ChatGPT.app"].find(app =>
  fs.existsSync(path.join(app, "Contents/Resources/app.asar")) &&
  ["codex", "codex-cli/bin/codex"].some(binary => fs.existsSync(path.join(app, "Contents/Resources", binary))));

// The App's file names carry content hashes and its code splitting changes between releases, so the gate
// is found by what it says, not by where it lives: a `didStart` method that tests `threadStartKind`.
const BUNDLE_ROOTS = ["webview/assets/", ".vite/build/"];
const GATE = /didStart\(\)\{[\s\S]{0,1200}?\.threadStartKind!=null/;
// The function that owns the gate takes its dependencies as one destructured parameter. Object keys survive
// minification, so the test binds the gate's free variables by key instead of by a minified name.
const GATE_DEPENDENCIES = /\{manager:[^{}]*onInitialTitleRequested:[^{}]*\}/g;
const GATE_DEPENDENCY_KEYS = ["manager", "conversationId", "operation", "origin", "onInitialTitleRequested"];
const MAX_BUNDLE_BYTES = 64 * 1024 * 1024;

function readGateBundles(app) {
  const file = fs.openSync(path.join(app, "Contents/Resources/app.asar"), "r");
  try {
    const prefix = Buffer.alloc(16);
    fs.readSync(file, prefix, 0, prefix.length, 0);
    const headerBytes = prefix.readUInt32LE(12);
    assert.ok(headerBytes > 0 && headerBytes < 32 * 1024 * 1024);
    const bytes = Buffer.alloc(headerBytes);
    fs.readSync(file, bytes, 0, bytes.length, 16);
    const header = JSON.parse(bytes.toString("utf8"));
    const dataStart = 8 + prefix.readUInt32LE(4);
    const found = [];
    const visit = (directory, parent) => {
      for (const [name, entry] of Object.entries(directory.files ?? {})) {
        const entryPath = `${parent}${name}`;
        if (entry.files) {
          visit(entry, `${entryPath}/`);
          continue;
        }
        if (!name.endsWith(".js") || entry.unpacked || entry.size > MAX_BUNDLE_BYTES) continue;
        if (!BUNDLE_ROOTS.some(root => entryPath.startsWith(root))) continue;
        const source = Buffer.alloc(entry.size);
        fs.readSync(file, source, 0, source.length, dataStart + Number(entry.offset));
        const text = source.toString("utf8");
        if (GATE.test(text)) found.push({ name: entryPath, source: text });
      }
    };
    visit(header, "");
    return found;
  } finally {
    fs.closeSync(file);
  }
}

test("the installed Codex first-turn gate requests native titles for unnamed default starts", {
  skip: installedApp == null ? "Codex Desktop is not installed on this host" : false,
}, async () => {
  // Evaluate only the installed App's small gate with a recording callback. No App connection,
  // title generator or model is invoked, and no proprietary App code is copied into this repo.
  const { default: ts } = await import("typescript");
  const bundles = readGateBundles(installedApp);
  assert.ok(bundles.length > 0, "the installed App no longer exposes the captured first-title gate");
  const input = [{ type: "text", text: "Fix the attachment button", text_elements: [] }];
  for (const { name, source } of bundles) {
    const at = GATE.exec(source).index;
    const dependencies = [...source.slice(Math.max(0, at - 65536), at).matchAll(GATE_DEPENDENCIES)].at(-1)?.[0];
    assert.ok(dependencies, `${name}: review the native first-title gate's captured dependency contract`);
    for (const key of GATE_DEPENDENCY_KEYS) {
      assert.match(dependencies, new RegExp(`[{,]${key}:`), `${name}: the first-title gate no longer receives \`${key}\``);
    }
    // The gate is a method of the object its function returns; parse it out of the text that follows.
    const fragment = ts.createSourceFile("native-title.js", `({${source.slice(at, at + 8192)}`, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
    let hook;
    const visit = node => {
      if (hook != null) return;
      if (ts.isMethodDeclaration(node) && node.name.getText(fragment) === "didStart") hook = node;
      else ts.forEachChild(node, visit);
    };
    visit(fragment);
    assert.ok(hook, `${name}: the first-title gate is no longer a method`);
    const factory = vm.runInNewContext(`(function(${dependencies}) { return { ${hook.getText(fragment)} }; })`);
    for (const [title, kind, role, origin, expected] of [
      [null, "default", "owner", "direct", 1],
      ["Task description used as a name", "default", "owner", "direct", 0],
      [null, undefined, "owner", "direct", 0],
      ["A name chosen by the user", "default", "owner", "direct", 0],
      [null, "default", "follower", "direct", 0],
      [null, "default", "owner", "nested", 0],
    ]) {
      const calls = [];
      const manager = { getConversation: () => ({ title }), getStreamRole: () => ({ role }), getHostId: () => "host" };
      const operation = { request: { input, cwd: "/project" }, context: { threadStartKind: kind, attachments: [] } };
      const { didStart } = factory({
        manager,
        conversationId: "thread",
        operation,
        origin,
        onInitialTitleRequested: event => calls.push(event),
        storage: {},
      });
      didStart.call(null);
      assert.equal(calls.length, expected, JSON.stringify({ name, title, kind, role, origin }));
      if (expected) {
        assert.equal(calls[0].conversationId, "thread");
        assert.equal(calls[0].input, input);
        assert.equal(calls[0].cwd, "/project");
      }
    }
  }
});
