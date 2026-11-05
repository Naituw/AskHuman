import Foundation
@testable import AHProtocol

// The Mac's side of away mode: it keeps a state, applies what the phone asks and pushes the new state.

extension TestMac {
    func sendAwayState(to phoneId: String) async {
        let state = lock.withLock {
            AwayState(rev: awayRev, global: awayGlobal, sessions: awaySessions, escalatedWaiting: awayEscalated)
        }
        await sendDown(state, to: phoneId)
    }

    func handleAway(_ plain: Plain, from phoneId: String) async {
        switch plain.t {
        case AwaySet.type:
            guard let set = try? plain.body(as: AwaySet.self) else { return }
            lock.withLock {
                awayGlobal = set.on ? AwayGlobal(sinceMs: clock.nowMs(), note: set.note, replies: 0) : nil
                awayRev += 1
            }
        case AwaySessionSet.type:
            guard let set = try? plain.body(as: AwaySessionSet.self) else { return }
            lock.withLock {
                awaySessions.removeAll { $0.sessionId == set.sessionId }
                if set.setting != AwaySessionSetting.follow.rawValue {
                    awaySessions.append(AwaySession(sessionId: set.sessionId, mode: set.setting, sinceMs: clock.nowMs(), note: set.note))
                }
                awayRev += 1
            }
        default:
            return
        }
        if answersAway { await sendAwayState(to: phoneId) }
    }

    /// Changes the state on the Mac's own (as the desktop does) and tells every paired phone.
    func changeAway(_ change: (inout AwayGlobal?, inout [AwaySession]) -> Void) async {
        lock.withLock {
            change(&awayGlobal, &awaySessions)
            awayRev += 1
        }
        for phone in pairedPhones { await sendAwayState(to: phone) }
    }
}
