import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

test("macOS session termination uses explicit quit reasons and the workspace notification", {
  skip: process.platform !== "darwin" && "requires AppKit",
}, (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "askhuman-session-end-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const main = path.join(root, "main.swift");
  fs.writeFileSync(main, String.raw`
import AppKit
import Carbon

func quit(_ reason: OSType?, attribute: Bool = false, eventID: AEEventID = AEEventID(kAEQuitApplication)) -> NSAppleEventDescriptor {
    let event = NSAppleEventDescriptor(eventClass: AEEventClass(kCoreEventClass), eventID: eventID,
        targetDescriptor: nil, returnID: AEReturnID(kAutoGenerateReturnID), transactionID: AETransactionID(kAnyTransactionID))
    if let reason {
        let value = NSAppleEventDescriptor(enumCode: reason)
        if attribute { event.setAttribute(value, forKeyword: AEKeyword(kAEQuitReason)) }
        else { event.setParam(value, forKeyword: AEKeyword(kAEQuitReason)) }
    }
    return event
}
precondition(!isSystemSessionQuit(nil))
precondition(!isSystemSessionQuit(quit(nil)))
precondition(!isSystemSessionQuit(quit(OSType(kAEQuitAll))))
precondition(!isSystemSessionQuit(quit(OSType(kAESleep))))
for reason in [OSType(kAEShutDown), OSType(kAERestart), OSType(kAEReallyLogOut)] {
    precondition(isSystemSessionQuit(quit(reason)))
    precondition(isSystemSessionQuit(quit(reason, attribute: true)))
    precondition(!isSystemSessionQuit(quit(reason, eventID: AEEventID(kAEOpenDocuments))))
}
var notices = 0
func ending() { notices += 1 }
var ordinaryQuits = 0
class Delegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        ordinaryQuits += 1
        return .terminateCancel
    }
}
let delegate = Delegate()
NSApplication.shared.delegate = delegate
observeSessionEnd(ending)
observeSessionEnd(ending)
precondition(NSApplication.shared.delegate?.applicationShouldTerminate?(NSApplication.shared) == .terminateCancel)
precondition(ordinaryQuits == 1)
let center = NSWorkspace.shared.notificationCenter
center.post(name: NSWorkspace.willSleepNotification, object: NSWorkspace.shared)
center.post(name: NSWorkspace.didWakeNotification, object: NSWorkspace.shared)
precondition(notices == 0)
center.post(name: NSWorkspace.willPowerOffNotification, object: NSWorkspace.shared)
precondition(notices == 1)
print("session termination checks passed")
`);
  const binary = path.join(root, "verify");
  const source = fileURLToPath(new URL("../src-tauri/swift/SessionEnd.swift", import.meta.url));
  const compile = spawnSync("xcrun", ["swiftc", source, main, "-o", binary], { encoding: "utf8", timeout: 60000 });
  assert.equal(compile.status, 0, compile.stderr || compile.error?.message);
  const result = spawnSync(binary, [], { encoding: "utf8", timeout: 10000 });
  assert.equal(result.status, 0, result.stderr || result.error?.message);
  assert.match(result.stdout, /session termination checks passed/);
});
