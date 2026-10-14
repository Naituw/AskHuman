import { h, nextTick } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
const mocks=vi.hoisted(()=>({ agent:vi.fn(), project:vi.fn(), terminal:vi.fn(), copy:vi.fn(), click:vi.fn() }));
vi.mock("../../lib/ipc",()=>({ popupAgentInfo:mocks.agent,popupProjectInfo:mocks.project,popupOpenTerminal:mocks.terminal }));
import SourcePopover from "./SourcePopover.vue";
describe("source information cards",()=>{
  const wrappers:VueWrapper[]=[];
  beforeEach(()=>{
    vi.useFakeTimers();vi.clearAllMocks();
    Object.defineProperty(navigator,"clipboard",{ configurable:true,value:{ writeText:mocks.copy.mockResolvedValue(undefined) } });
    mocks.agent.mockResolvedValue({ model:"current-model",reasoning:"high",context:{usedTokens:20000,windowTokens:200000,remainingPercent:90,estimated:false} });
    mocks.project.mockResolvedValue({ branch:"main",detached:false,changes:{total:3,staged:1,unstaged:1,untracked:1} });
    mocks.terminal.mockResolvedValue(undefined);
  });
  afterEach(()=>{wrappers.forEach(w=>w.unmount());wrappers.length=0;vi.useRealTimers();document.body.innerHTML="";});
  function start(kind:"agent"|"project"="agent") {
    const wrapper=mount(SourcePopover,{attachTo:document.body,props:{kind,requestId:"request-a",active:true,label:kind==="agent"?"Codex":"Repo",sessionId:"exact-session",sessionTitle:"Native task name",project:"/repo/a",callerCwd:"/tmp/report"},
      slots:{default:()=>h("button",{class:"brand-chip",onClick:mocks.click},kind==="agent"?"Codex":"Repo")},global:{plugins:[createI18n({legacy:false,locale:"en",messages:{en}})]}});
    wrappers.push(wrapper);return wrapper;
  }
  async function hover(wrapper:VueWrapper) { await wrapper.find(".source-popover-anchor").trigger("mouseenter");await vi.advanceTimersByTimeAsync(100);await flushPromises(); }
  it("waits exactly 100ms, displays exact session facts in a floating card and keeps the original click",async()=>{
    const wrapper=start();await wrapper.find(".source-popover-anchor").trigger("mouseenter");await vi.advanceTimersByTimeAsync(99);
    expect(document.querySelector('[role="dialog"]')).toBeNull();expect(mocks.agent).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);await flushPromises();
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("Native task name");
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("current-model");
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(mocks.agent).toHaveBeenCalledWith("request-a");expect(mocks.project).not.toHaveBeenCalled();
    await wrapper.find("button").trigger("click");expect(mocks.click).toHaveBeenCalledOnce();
  });
  it("copies the full identity and closes on Escape without cancelling the question",async()=>{
    const cancel=vi.fn();window.addEventListener("keydown",cancel);
    const wrapper=start();await hover(wrapper);
    (document.querySelector('[aria-label="Copy session ID"]') as HTMLButtonElement).click();await flushPromises();
    expect(mocks.copy).toHaveBeenCalledWith("exact-session");
    window.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));await nextTick();
    expect(document.querySelector('[role="dialog"]')).toBeNull();expect(cancel).not.toHaveBeenCalled();window.removeEventListener("keydown",cancel);
  });
  it("puts paths and Git in the project card and restricts terminal actions to known targets",async()=>{
    const wrapper=start("project");await hover(wrapper);
    const card=document.querySelector('[role="dialog"]')!;
    expect(card.textContent).toContain("/repo/a");expect(card.textContent).toContain("/tmp/report");expect(card.textContent).toContain("main");
    expect(card.textContent).not.toContain("exact-session");expect(mocks.agent).not.toHaveBeenCalled();
    const terminal=card.querySelectorAll<HTMLButtonElement>('[aria-label="Open in terminal"]');terminal[1].click();await flushPromises();
    expect(mocks.terminal).toHaveBeenCalledWith("request-a","caller");
    card.querySelector<HTMLButtonElement>('[aria-label="Copy path"]')!.click();await flushPromises();expect(mocks.copy).toHaveBeenCalledWith("/repo/a");
  });
  it("does not poll an open card and drops a late result when the request is hidden",async()=>{
    const wrapper=start();await hover(wrapper);await vi.advanceTimersByTimeAsync(5000);expect(mocks.agent).toHaveBeenCalledOnce();
    await wrapper.setProps({active:false});expect(document.querySelector('[role="dialog"]')).toBeNull();
    await wrapper.setProps({active:true});let resolve!:(value:unknown)=>void;mocks.agent.mockReturnValueOnce(new Promise(r=>{resolve=r;}));
    await wrapper.find(".source-popover-anchor").trigger("mouseenter");await vi.advanceTimersByTimeAsync(100);
    await wrapper.setProps({active:false});resolve({model:"late-wrong-request"});await flushPromises();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });
  it("omits unavailable fields and only keeps one source card open",async()=>{
    mocks.agent.mockResolvedValue({});const a=start(),p=start("project");await hover(a);
    expect(document.querySelector('[role="dialog"]')?.textContent).not.toContain("Model");
    await hover(p);expect(document.querySelectorAll('[role="dialog"]').length).toBe(1);
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("/repo/a");
  });
});
