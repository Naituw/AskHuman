import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const scripts = fileURLToPath(new URL('../.agents/skills/ios-testflight/scripts/', import.meta.url));
test('external mutations stay within the app and gate delivery on actual approval', {
  skip: process.platform !== 'darwin' && 'requires Swift on macOS',
}, (t) => {
  const folder = mkdtempSync(join(tmpdir(), 'testflight-external-mutations-'));
  t.after(() => rmSync(folder, { recursive: true, force: true }));
  const binary = join(folder, 'checks');
  const compile = spawnSync('xcrun', ['swiftc', '-parse-as-library',
    ...['ExternalStatus.swift', 'ExternalTesting.swift', 'ExternalTestingTests.swift'].map(name => join(scripts, name)),
    '-o', binary], { encoding: 'utf8' });
  assert.equal(compile.status, 0, compile.stderr);
  const run = spawnSync(binary, [], { encoding: 'utf8' });
  assert.equal(run.status, 0, run.stderr);
});
test('external inspection separates processing from approval and limits contact recovery', {
  skip: process.platform !== 'darwin' && 'requires Swift on macOS',
}, (t) => {
  const folder = mkdtempSync(join(tmpdir(), 'testflight-external-'));
  t.after(() => rmSync(folder, { recursive: true, force: true }));
  const binary = join(folder, 'checks');
  const compile = spawnSync('xcrun', ['swiftc', '-parse-as-library', join(scripts, 'ExternalStatus.swift'),
    join(scripts, 'DistributionRules.swift'), join(scripts, 'ExternalStatusTests.swift'), '-o', binary], { encoding: 'utf8' });
  assert.equal(compile.status, 0, compile.stderr);
  const run = spawnSync(binary, [], { encoding: 'utf8' });
  assert.equal(run.status, 0, run.stderr);
});
