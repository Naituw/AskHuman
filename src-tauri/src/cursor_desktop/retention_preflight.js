(function verifyRetention(context) {
  "use strict";
  // Drives the production wrapper over a virtual service and a counting stand-in for Cursor's own
  // unload method. Nothing here touches a real conversation, listener, or Cursor prototype.
  const { create } = context;
  let checks = 0;
  const check = (condition, name) => {
    if (!condition) throw new Error(`Retention preflight failed: ${name}`);
    checks++;
  };
  const marker = { native: true };
  const nativeFailure = new Error("native unload failed");
  const calls = [];
  let mode = "return";
  const original = function (...args) {
    calls.push({ self: this, args });
    if (mode === "throw") throw nativeFailure;
    return marker;
  };
  const { wrapper, control } = create(original);
  const service = {
    getComposerData(handle) {
      if (handle.unreadable) throw new Error("composer data is unreadable");
      return handle.data;
    },
    _composerDataIsBusy: data => data.applyingCode === true,
  };
  const chat = (data, extra) => ({ data: { conversationMap: {}, generatingBubbleIds: [], ...data }, ...extra });
  const loaded = count => Object.fromEntries(Array.from({ length: count }, (_, index) => [`bubble-${index}`, {}]));
  const ids = ["first", "second"];

  // A running chat keeps its messages, whichever way Cursor marks it as running.
  const running = [
    chat({ status: "generating", conversationMap: loaded(5) }),
    chat({ status: "completed", generatingBubbleIds: ["live"], conversationMap: loaded(5) }),
    chat({ status: "completed", applyingCode: true, conversationMap: loaded(2) }),
  ];
  const results = running.map(handle => wrapper.call(service, handle, ids, "context"));
  check(calls.length === 0 && results.every(value => value === undefined), "running chats keep their messages");
  check(control.skipped === 3 && control.skippedBubbles === 6 && control.passed === 0, "skips are counted");
  check(control.recent.length === 3 && control.recent.map(entry => entry.scope).join() === "partial,partial,all" &&
    control.recent.every(entry => Object.keys(entry).sort().join() === "at,bubbles,scope"), "recent skips are anonymous");

  // Any other chat reaches Cursor's own method with the same receiver, arguments, and result.
  const idle = chat({ status: "completed" });
  const outcome = wrapper.call(service, idle, ids, "context", "extra");
  check(outcome === marker && calls.length === 1 && calls[0].self === service && calls[0].args.length === 4 &&
    calls[0].args[0] === idle && calls[0].args[1] === ids && calls[0].args[3] === "extra", "idle chats unload natively");
  check(control.passed === 1 && control.skipped === 3, "pass-through is counted separately");

  // A chat that cannot be read is never blocked, and the guard never throws by itself.
  calls.length = 0;
  const unreadable = { data: {}, unreadable: true };
  check(wrapper.call(service, unreadable, ids) === marker && calls.length === 1, "an unreadable chat falls through");
  check(wrapper.call(service, undefined, ids) === marker && calls.length === 2, "a missing chat falls through");

  // Native failures are not hidden.
  mode = "throw";
  let thrown;
  try { wrapper.call(service, idle, ids); } catch (error) { thrown = error; }
  check(thrown === nativeFailure, "native errors propagate unchanged");
  mode = "return";

  // A disabled guard is invisible even for a running chat.
  control.enabled = false;
  calls.length = 0;
  check(wrapper.call(service, running[0], ids) === marker && calls.length === 1, "a disabled guard defers to Cursor");
  control.enabled = true;

  // The recent list is bounded.
  for (let count = 0; count < 40; count++) wrapper.call(service, running[0], ids);
  check(control.recent.length === 20, "recent skips stay bounded");

  return { passed: true, checks };
})
