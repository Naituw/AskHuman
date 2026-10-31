import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { assertDevTarget, sourceFingerprint, recordBuild } from './dev-build-record.mjs';

function fixture(t) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'askhuman-build-record-')));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const dir of ['src', '.askhuman-dev/bin', '.askhuman-dev/home']) fs.mkdirSync(path.join(root, dir), { recursive: true });
  fs.writeFileSync(path.join(root, 'src/main.ts'), 'export const version = 1;\n');
  const binary = path.join(root, '.askhuman-dev/bin/AskHuman');
  fs.writeFileSync(binary, 'installed binary');
  return { root, binary };
}

test('records installed bytes and rejects edits made during the build', (t) => {
  const { root, binary } = fixture(t);
  const before = sourceFingerprint(root);
  const record = recordBuild(root, binary, before);
  assert.equal(record.sourceFingerprint, before);
  assert.equal(record.binary, fs.realpathSync(binary));
  fs.writeFileSync(path.join(root, 'src/main.ts'), 'export const version = 2;\n');
  assert.throws(() => recordBuild(root, binary, before), /changed during installation/);
  assert.equal(fs.existsSync(path.join(root, '.askhuman-dev/build.json')), false);
});

test('includes added files and ignores generated output and instance state', (t) => {
  const { root } = fixture(t);
  const first = sourceFingerprint(root);
  fs.writeFileSync(path.join(root, '.askhuman-dev/home/config.json'), '{}');
  fs.mkdirSync(path.join(root, 'src/node_modules'), { recursive: true });
  fs.writeFileSync(path.join(root, 'src/node_modules/generated.ts'), 'generated');
  assert.equal(sourceFingerprint(root), first);
  fs.writeFileSync(path.join(root, 'src/added.ts'), 'new source');
  assert.notEqual(sourceFingerprint(root), first);
});

test('rejects an unbuilt change to the embedded macOS Info.plist', (t) => {
  const { root, binary } = fixture(t);
  fs.mkdirSync(path.join(root, 'src-tauri'));
  const plist = path.join(root, 'src-tauri/Info.plist');
  fs.writeFileSync(plist, '<plist>original permissions</plist>');
  const before = sourceFingerprint(root);
  recordBuild(root, binary, before);
  fs.writeFileSync(plist, '<plist>changed permissions</plist>');
  assert.notEqual(sourceFingerprint(root), before);
  assert.throws(() => recordBuild(root, binary, before), /changed during installation/);
  assert.equal(fs.existsSync(path.join(root, '.askhuman-dev/build.json')), false);
});

test('rejects binary aliases outside the instance, including dangling aliases', { skip: process.platform === 'win32' }, (t) => {
  const { root, binary } = fixture(t);
  fs.unlinkSync(binary);
  fs.symlinkSync(path.join(root, 'production-binary'), binary);
  assert.throws(() => assertDevTarget(root, binary), /outside/);
  fs.writeFileSync(path.join(root, 'production-binary'), 'production');
  assert.throws(() => assertDevTarget(root, binary), /outside/);
});

test('rejects a home that points outside the instance', { skip: process.platform === 'win32' }, (t) => {
  const { root, binary } = fixture(t);
  fs.rmdirSync(path.join(root, '.askhuman-dev/home'));
  fs.symlinkSync(root, path.join(root, '.askhuman-dev/home'));
  assert.throws(() => assertDevTarget(root, binary), /home escapes/);
});
