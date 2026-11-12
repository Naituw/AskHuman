import Foundation
import CoreGraphics
import UniformTypeIdentifiers
import AHProtocol
import AHCore
import AHStore

// Sample content of the `historyPictures` scenario (docs/specs/ios-history-pictures.md): a session whose exchanges carry
// pictures of every kind the cards must show, and a finished request of the History tab whose pictures the Mac still has.

extension MockData {
    /// The key of the session whose exchanges carry pictures.
    public static let picturesSessionKey = "claude:mock-pictures-session"
    /// The request of the pictures session that waits in the inbox, so that its page has the thread.
    public static let picturesRequestId = "req-pictures-now"
    /// The request the History tab's detail page shows, and the id of the exchange the Mac holds for it.
    public static let picturesHistoryRequestId = "ex-pics-05"

    // MARK: pictures

    /// A picture of a history exchange, as the Mac describes it: a numbered picture of one hue whose thumbnail keeps the
    /// shape of the original (the Mac's does too). `gone` is a file the Mac no longer has: no thumbnail, nothing to fetch.
    /// The id carries what the mock draws (`hpic-{number}-{width}x{height}-{hue %}`), so that a download needs no table.
    public static func historyPicture(
        _ number: Int, name: String? = nil, width: Int = 1200, height: Int = 800, hue: Double = 0.58, gone: Bool = false
    ) -> AssetRef {
        let step = Int((hue * 100).rounded())
        let id = "hpic-\(number)-\(width)x\(height)-\(step)"
        let name = name ?? "screen-\(number).png"
        if gone { return AssetRef(assetId: id, name: name, size: 0, mime: "image/png", isImage: true, gone: true) }
        let edge = 160.0 / Double(max(width, height))
        let thumb = picture(
            number: number, width: max(1, Int(Double(width) * edge)), height: max(1, Int(Double(height) * edge)),
            hue: CGFloat(step) / 100, type: .jpeg
        )
        return AssetRef(
            assetId: id, name: name, size: UInt64(120_000 + number * 40_000), mime: "image/png", isImage: true,
            thumb: thumb.map(Bytes.b64u)
        )
    }

    /// What `historyPicture` drew, read back from the id.
    static func historyPictureShape(of assetId: String) -> (number: Int, width: Int, height: Int, hue: CGFloat)? {
        let parts = assetId.split(separator: "-")
        guard parts.count == 4, parts[0] == "hpic", let number = Int(parts[1]), let step = Int(parts[3]) else { return nil }
        let size = parts[2].split(separator: "x").compactMap { Int($0) }
        guard size.count == 2 else { return nil }
        return (number, size[0], size[1], CGFloat(step) / 100)
    }

    // MARK: the session

    /// The pictures session, newest first: seven pictures (and a file that is not one), a single one, two of which one is
    /// gone, two that are both gone, and the answer to a question that carries two pictures. Every kind of shape is
    /// among them.
    public static func picturesSessionExchanges(now: Date = Date()) -> [SessionExchange] {
        func at(_ index: Int) -> Int64 { ms(now) - 300_000 - Int64(index) * 3_600_000 }
        let onboarding: [AssetRef] = [
            historyPicture(1, name: "onboarding-1.png", width: 1200, height: 800, hue: 0.58),
            historyPicture(2, name: "onboarding-2.png", width: 800, height: 1200, hue: 0.08),
            historyPicture(3, name: "onboarding-3.png", width: 1000, height: 1000, hue: 0.33),
            historyPicture(4, name: "onboarding-4.png", width: 2400, height: 600, hue: 0.80),
            historyPicture(5, name: "onboarding-5.png", width: 1200, height: 800, hue: 0.15),
            historyPicture(6, name: "onboarding-6.png", width: 800, height: 1200, hue: 0.45),
            historyPicture(7, name: "onboarding-7.png", width: 1000, height: 1000, hue: 0.70),
        ]
        return [
            SessionExchange(
                id: picturesHistoryRequestId, at: at(0), channel: "popup", markdown: true,
                message: "Seven screenshots of the onboarding flow, and the checklist.", attachments: 8, pictures: onboarding,
                qs: [SessionQuestion(text: "Which step should I polish first? #5", input: "Step three")]
            ),
            SessionExchange(
                id: "ex-pics-04", at: at(1), channel: "ios", markdown: true,
                message: "One screenshot of the login screen.", attachments: 1,
                pictures: [historyPicture(1, name: "login.png", width: 800, height: 1200, hue: 0.62)],
                qs: [SessionQuestion(text: "Is this the right colour? #4", selected: ["Yes, ship it"])]
            ),
            SessionExchange(
                id: "ex-pics-03", at: at(2), channel: "dingding", markdown: true,
                message: "Two screenshots, and one of them is already cleaned up.", attachments: 2,
                pictures: [
                    historyPicture(1, name: "settings.png", width: 1200, height: 800, hue: 0.25),
                    historyPicture(2, name: "cleaned-up.png", width: 1200, height: 800, hue: 0.9, gone: true),
                ],
                qs: [SessionQuestion(text: "Which of the two do you prefer? #3", input: "The first")]
            ),
            SessionExchange(
                id: "ex-pics-02", at: at(3), channel: "popup", markdown: true,
                message: "截图都已经被系统清理掉了。", attachments: 2,
                pictures: [
                    historyPicture(1, name: "old-1.png", hue: 0.1, gone: true),
                    historyPicture(2, name: "old-2.png", hue: 0.2, gone: true),
                ],
                qs: [SessionQuestion(text: "还需要我重新截图吗？ #2", selected: ["不用了"])]
            ),
            SessionExchange(
                id: "ex-pics-01", at: at(4), channel: "popup", markdown: true,
                message: "I will make the header look the way you describe it.",
                qs: [
                    SessionQuestion(
                        text: "Like this? #1", input: "Make it look like these two", images: 2,
                        pictures: [
                            historyPicture(11, name: "reply-1.png", width: 1000, height: 1000, hue: 0.5),
                            historyPicture(12, name: "reply-2.png", width: 1200, height: 800, hue: 0.95),
                        ]
                    ),
                ]
            ),
        ]
    }

    /// The inbox of the scenario: the requests of `sessionHistory`, and one of the pictures session.
    public static func historyPicturesInbox(now: Date = Date()) -> [InboxItem] {
        var all = sessionHistoryInbox(now: now)
        all += items([
            (homeMacId, "Home MacBook Pro",
             sessionHistoryAsk(id: picturesRequestId, key: picturesSessionKey, title: "Polish the onboarding screens", now: now), true, -20),
        ], now: now)
        return all
    }

    // MARK: the History tab

    /// A finished request of the History tab whose message had the seven pictures of `picturesHistoryRequestId`, and a
    /// file that is not one. The phone kept only the names; the Mac still has the pictures.
    public static func picturesHistoryEntry(now: Date = Date()) -> HistoryEntry {
        let names = (1...7).map { "onboarding-\($0).png" }
        let images = names.enumerated().map { index, name in
            AssetRef(assetId: "req-file-\(index + 1)", name: name, size: UInt64(120_000 + index * 40_000), mime: "image/png", isImage: true)
        }
        let checklist = AssetRef(assetId: "req-file-8", name: "checklist.md", size: 1_204, mime: "text/markdown", isImage: false)
        var request = sessionHistoryAsk(
            id: picturesHistoryRequestId, key: picturesSessionKey, title: "Polish the onboarding screens",
            message: "Seven screenshots of the onboarding flow, and the checklist.", now: now.addingTimeInterval(-600)
        )
        request.ask?.message.files = images + [checklist]
        request.ask?.questions = [
            QuestionView(text: "Which step should I polish first?", options: [
                OptionView(text: "Step one", recommended: false), OptionView(text: "Step three", recommended: true),
            ]),
        ]
        return HistoryEntry(
            id: CachedRequest.key(homeMacId, request.id), macId: homeMacId, macName: "Home MacBook Pro", request: request,
            outcome: .answered, answers: [AnswerItem(q: 0, sel: [1], text: "Step three")],
            receivedAt: now.addingTimeInterval(-600), finishedAt: now.addingTimeInterval(-540)
        )
    }
}
