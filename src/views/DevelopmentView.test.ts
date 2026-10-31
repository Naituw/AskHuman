import { mount, flushPromises } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { i18n } from '../i18n';
import DevelopmentTab from './DevelopmentView.vue';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
const status = {
  systemBinary: '/system/AskHuman', instanceTarget: { kind: 'dev' },
  worktrees: [{ root: '/work/tree-a', name: 'tree-a', binary: '/work/tree-a/.askhuman-dev/bin/AskHuman', home: '/work/tree-a/.askhuman-dev/home', enabled: true, build: { createdAtMs: 1000, binaryFingerprint: 'build-a' }, error: null, preferences: { delivery: 'local' } }],
  agents: [{ kind: 'codex', supported: true, reason: null }, { kind: 'cursor', supported: false, reason: 'global MCP cannot be isolated' }],
};

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation(async (command) => {
    if (command === 'development_status') return structuredClone(status);

  });
});
async function render() {
  const wrapper = mount(DevelopmentTab, { global: { plugins: [i18n] } });
  await flushPromises();
  return wrapper;
}
function button(wrapper: Awaited<ReturnType<typeof render>>, text: RegExp) {
  const match = wrapper.findAll('button').find((item) => text.test(item.text()));
  if (!match) throw new Error(`missing button ${text}`);
  return match;
}

describe('development workbench isolation', () => {
  it('only inspects its bound Dev and offers no global migration or registration', async () => {
    const wrapper = await render();
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(['development_status']);
    expect(wrapper.findAll('input')).toHaveLength(0);
    expect(wrapper.text()).toContain('/work/tree-a');
    wrapper.unmount();
  });
  it('launches the selected worktree and disables unsupported native clients', async () => {
    const wrapper = await render();
    const selects = wrapper.findAll('select');
    await selects[1]!.setValue('cursor');
    expect(button(wrapper, /Start test session|启动测试会话/).attributes('disabled')).toBeDefined();
    expect(wrapper.text()).toContain('global MCP cannot be isolated');
    await selects[1]!.setValue('codex');
    await button(wrapper, /Start test session|启动测试会话/).trigger('click');
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith('development_start_session', expect.objectContaining({ worktree: '/work/tree-a', agent: 'codex' }));
    wrapper.unmount();
  });
  it('opens an explicit Dev window and restores the delivery choice on failure', async () => {
    const wrapper = await render();
    await button(wrapper, /Dev history|Dev 历史|测试历史/).trigger('click');
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith('development_open_window', { worktree: '/work/tree-a', kind: 'history' });
    invoke.mockRejectedValueOnce(new Error('cannot save'));
    const delivery = wrapper.findAll('select')[0]!;
    await delivery.setValue('remote');
    await flushPromises();
    expect((delivery.element as HTMLSelectElement).value).toBe('local');
    expect(wrapper.text()).toContain('cannot save');
    wrapper.unmount();
  });
});
