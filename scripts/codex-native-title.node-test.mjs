import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { test } from "node:test";

const installedApp = ["/Applications/Codex.app", "/Applications/ChatGPT.app"].find(app =>
  fs.existsSync(path.join(app, "Contents/Resources/app.asar")) &&
  ["codex", "codex-cli/bin/codex"].some(binary => fs.existsSync(path.join(app, "Contents/Resources", binary))));

function readNativeEntry(app) {
  const file = fs.openSync(path.join(app, "Contents/Resources/app.asar"), "r");
  try {
    const prefix = Buffer.alloc(16);
    fs.readSync(file, prefix, 0, prefix.length, 0);
    const headerBytes = prefix.readUInt32LE(12);
    assert.ok(headerBytes > 0 && headerBytes < 32 * 1024 * 1024);
    const bytes = Buffer.alloc(headerBytes);
    fs.readSync(file, bytes, 0, bytes.length, 16);
    const header = JSON.parse(bytes.toString("utf8"));
    const assets = header.files.webview.files.assets.files;
    const name = Object.keys(assets).find(name => /^app-initial-.*\.js$/.test(name));
    assert.ok(name, "review the native title contract after a Codex App bundle change");
    const entry = assets[name];
    const source = Buffer.alloc(entry.size);
    fs.readSync(file, source, 0, source.length, 8 + prefix.readUInt32LE(4) + Number(entry.offset));
    return source.toString("utf8");
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
  const source = readNativeEntry(installedApp);
  const condition = /didStart\(\)\{[^}]*\.threadStartKind!=null/.exec(source)?.index ?? -1;
  assert.ok(condition >= 0, "the installed App no longer exposes the captured first-title gate");
  const start = source.lastIndexOf("function ", condition);
  const fragment = ts.createSourceFile("native-title.js", source.slice(start, start + 16384), ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const declaration = fragment.statements.find(ts.isFunctionDeclaration);
  assert.ok(declaration);
  const names = declaration.parameters.map(parameter => parameter.name.getText(fragment));
  assert.equal(names.length, 8, "review the native first-title gate's captured parameter contract");
  let hook;
  const visit = node => {
    if (ts.isMethodDeclaration(node) && node.name.getText(fragment) === "didStart") hook = node;
    ts.forEachChild(node, visit);
  };
  visit(declaration);
  assert.ok(hook);
  const factory = vm.runInNewContext(`(function(${names.join(",")}) { return function ${hook.getText(fragment)}; })`);
  const input = [{ type: "text", text: "Fix the attachment button", text_elements: [] }];
  for (const [title, kind, role, origin, expected] of [
    [null, "default", "owner", "direct", 1],
    ["Task description used as a name", "default", "owner", "direct", 0],
    [null, undefined, "owner", "direct", 0],
    ["A name chosen by the user", "default", "owner", "direct", 0],
    [null, "default", "follower", "direct", 0],
    [null, "default", "owner", "nested", 0],
  ]) {
    const calls = [];
    const manager = { getConversation: () => ({ title }), getStreamRole: () => ({ role }) };
    const operation = { request: { input, cwd: "/project" }, context: { threadStartKind: kind, attachments: [] } };
    factory(manager, "thread", operation, "message", origin, {}, event => calls.push(event), () => {}).call(null);
    assert.equal(calls.length, expected, JSON.stringify({ title, kind, role, origin }));
    if (expected) {
      assert.equal(calls[0].conversationId, "thread");
      assert.equal(calls[0].input, input);
      assert.equal(calls[0].cwd, "/project");
    }
  }
});
