// Tests for the pure parts of probe.mjs (argument rules, entrance discovery, and judging a burst).
// The network part is exercised by running the probe against production.
// Run: node --test scripts/relay-release-*.node-test.mjs (also part of `npm test`).
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseArgs, entrancesFromSources, parseHealth, judgeBurst, MAX_N, DEFAULT_N } from '../.agents/skills/relay-release/scripts/probe.mjs';

const ok = (ms) => ({ status: 200, ms });
const burst = (spec) => spec.flatMap(([status, count, ms = 100]) => Array.from({ length: count }, () => ({ status, ms })));

test('arguments: defaults, repeated --url, and the cap on --n', () => {
  assert.deepEqual(parseArgs([]), { urls: [], n: DEFAULT_N });
  assert.deepEqual(parseArgs(['--url', 'https://a.example', '--url', 'https://b.example/x', '--n', '8']), { urls: ['https://a.example', 'https://b.example/x'], n: 8 });
  assert.throws(() => parseArgs(['--n', String(MAX_N + 1)]), /--n must be/);
  assert.throws(() => parseArgs(['--n', '0']), /--n must be/);
  assert.throws(() => parseArgs(['--url', 'http://a.example']), /https URL/);
  assert.throws(() => parseArgs(['--what']), /unknown argument/);
});

test('entrances: the default URL from config.rs plus the explicit old URL from the config', () => {
  const configRs = 'pub const IOS_DEFAULT_RELAY_URL: &str = "https://relay.example.com";';
  const configJson = JSON.stringify({ channels: { ios: { relayUrl: 'https://123456789-abcdef.ap-hongkong.tencentscf.com/' } } });
  const e = entrancesFromSources({ configRs, configJson });
  assert.deepEqual(e.map((x) => x.url), ['https://relay.example.com', 'https://123456789-abcdef.ap-hongkong.tencentscf.com']);
});

test('entrances: a missing, empty or identical old URL fails instead of being skipped', () => {
  const configRs = 'pub const IOS_DEFAULT_RELAY_URL: &str = "https://relay.example.com";';
  for (const configJson of ['', '{}', JSON.stringify({ channels: { ios: { relayUrl: '' } } }), JSON.stringify({ channels: { ios: { relayUrl: 'https://relay.example.com/' } } })]) {
    assert.throws(() => entrancesFromSources({ configRs, configJson }), /cannot find the old function URL/);
  }
  assert.throws(() => entrancesFromSources({ configRs: '', configJson: '{}' }), /IOS_DEFAULT_RELAY_URL/);
});

test('health: needs 200 and both ok and apns true', () => {
  assert.equal(parseHealth({ status: 200, body: '1d\r\n{"ok":true,"v":1,"apns":true}\r\n0\r\n\r\n' }).ok, true);
  assert.equal(parseHealth({ status: 200, body: '{"ok":true,"v":1,"apns":false}' }).ok, false);
  assert.equal(parseHealth({ status: 200, body: 'not json' }).ok, false);
  assert.match(parseHealth({ status: 432, body: '' }).why, /432/);
  assert.match(parseHealth({ status: 0, error: 'timeout', body: '' }).why, /timeout/);
});

test('burst: all 200 passes', () => {
  const j = judgeBurst(burst([[200, 12]]), 12);
  assert.equal(j.ok, true);
  assert.deepEqual(j.counts, { 200: 12 });
});

test('burst: the concurrency-1 failure (2 slots) is stopped and explained', () => {
  const j = judgeBurst(burst([[200, 2], [432, 10]]), 12);
  assert.equal(j.ok, false);
  assert.match(j.why, /10 refused with 432/);
  assert.match(j.why, /concurrent slots/);
});

test('burst: a single refusal, a timeout or a missing request is a failure', () => {
  assert.equal(judgeBurst(burst([[200, 11], [432, 1]]), 12).ok, false);
  assert.equal(judgeBurst([...burst([[200, 11]]), { status: 0, error: 'timeout', ms: 20000 }], 12).ok, false);
  assert.match(judgeBurst([...burst([[200, 11]]), { status: 0, error: 'timeout', ms: 20000 }], 12).why, /error:timeout x1/);
  assert.equal(judgeBurst(burst([[200, 11]]), 12).ok, false);
});

test('burst: other statuses are reported', () => {
  const j = judgeBurst(burst([[200, 11], [502, 1]]), 12);
  assert.equal(j.ok, false);
  assert.match(j.why, /502 x1/);
});
