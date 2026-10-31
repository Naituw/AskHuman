import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { test } from 'node:test';
const template = fs.readFileSync(new URL('../src-tauri/src/cursor_desktop/message_renderer.js', import.meta.url), 'utf8');
function fixture({ busy = false, race = false, noProof = false, failure = false } = {}) {
  let calls = 0, released = 0;
  const handle = { data: {} };
  const chat = {
    async acquirePreGenerationSubmission() { if (race) busy = true; return () => { released++; }; },
    isQuietForBackgroundSubmit: () => !busy,
    _submitChatMaybeAbortCurrent(id, text, options) {
      calls++;
      if (failure) throw Error('uncertain native acceptance');
      if (!noProof) handle.data.chatGenerationUUID = 'generation';
      options._internalOnGenerationClaimed();
      return Promise.resolve();
    },
  };
  const data = { _instantiationService: { _services: { _entries: [['composerChatService', chat]] } }, getHandleIfLoaded: id => id === 'original' ? handle : undefined };
  const context = vm.createContext({ setTimeout, clearTimeout, data });
  const source = template.replace('__ASKHUMAN_OWNER__', '"home"').replace('__ASKHUMAN_REVISION__', '"revision"').replace('__ASKHUMAN_LEASE__', '"lease"');
  vm.runInContext(`(function(){ return ${source}; }).call(data)`, context);
  return { state: context.__ASKHUMAN_CURSOR_MESSAGES_V1__, calls: () => calls, released: () => released };
}
const op = { sessionId: 'original', id: 'one', text: 'Continue the task' };
test('native generation proof and duplicate requests share a single submission', async () => {
  const f = fixture();
  const [a, b] = await Promise.all([f.state.send(op), f.state.send(op)]);
  assert.equal(a.status, 'started'); assert.equal(a.turnId, 'generation'); assert.equal(b.turnId, 'generation');
  assert.equal(f.calls(), 1); assert.equal(f.released(), 1);
  assert.equal((await f.state.send({ ...op, text: 'changed' })).reason, 'conflict');
});
test('busy sessions and the pre-generation lock race never submit or abort', async () => {
  for (const flags of [{ busy: true }, { race: true }]) {
    const f = fixture(flags);
    assert.equal((await f.state.send(op)).status, 'rejected'); assert.equal(f.calls(), 0); assert.equal(f.released(), 1);
  }
});
test('only the original loaded composer can receive a message', async () => {
  const f = fixture(); assert.equal(f.state.owns('other'), false);
  assert.equal((await f.state.send({ ...op, sessionId: 'other' })).reason, 'notFound'); assert.equal(f.calls(), 0);
});
test('a claimed generation without its identity and a native exception remain unknown', async () => {
  for (const flags of [{ noProof: true }, { failure: true }]) {
    const f = fixture(flags); assert.equal((await f.state.send(op)).status, 'unknown');
    assert.equal((await f.state.send(op)).status, 'unknown'); assert.equal(f.calls(), 1); assert.equal(f.released(), 1);
  }
});

test('the independent authenticated message bridge template is valid JavaScript', () => {
  const original = fs.readFileSync(new URL('../src-tauri/src/cursor_desktop/bridge.js', import.meta.url), 'utf8');
  const dispatch = fs.readFileSync(new URL('../src-tauri/src/cursor_desktop/message_dispatch.js', import.meta.url), 'utf8');
  const bridge = original.replaceAll('__ASKHUMAN_CURSOR_BRIDGE_', '__ASKHUMAN_CURSOR_MESSAGE_BRIDGE_')
    .replaceAll('__ASKHUMAN_CURSOR_BACKGROUND_V1__', '__ASKHUMAN_CURSOR_MESSAGES_V1__')
    .replaceAll('Buffer.byteLength(buffer) > 4096', 'Buffer.byteLength(buffer) > 262144')
    .replace('else if (request.op === "restore")', `${dispatch}\nelse if (request.op === "restore")`);
  assert.equal(bridge.split('request.op === "message"').length, 2);
  new vm.Script(bridge);
});
