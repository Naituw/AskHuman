import Foundation

// Rules for the helper's `notes` operation, which writes the TestFlight "What to Test" text.
//
// This file is pure logic: it reads no Keychain item and opens no network connection, so it is compiled
// into the signed helper and, separately, into NotesRulesTests.swift. `notes-rules-cases.json` is the
// contract shared with `notes_*` in helper.py; keep both implementations in step with it.

/// A rejected request. `reason` is a stable code that the shared cases assert.
struct NotesRejection: Error, CustomStringConvertible {
    let reason: String
    let description: String
    init(_ reason: String, _ message: String) {
        self.reason = reason
        description = message
    }
}

enum NotesMode: String {
    case read, write, probe
}

struct NotesRequest: Equatable {
    let mode: NotesMode
    let build: String
    /// Canonical text by locale; empty unless the mode is `write`.
    let texts: [String: String]
}

struct RemoteNote: Equatable {
    let id: String
    let text: String
}

enum NotesAction: Equatable {
    case unchanged
    case create
    case update(id: String)
}

struct NotesStep: Equatable {
    let locale: String
    let action: NotesAction
}

/// Decides which requests the notes operation may send. Every request goes through `authorize`.
final class NotesGate {
    static let builds = "/v1/builds"
    static let collection = "/v1/betaBuildLocalizations"

    let mode: NotesMode
    /// Localization IDs listed for the target build or created by this run: the only ones a PATCH may name.
    private(set) var known = Set<String>()
    /// Localization IDs created by this run: the only ones a probe may delete.
    private(set) var created = Set<String>()

    init(mode: NotesMode) {
        self.mode = mode
    }

    static func isIdentifier(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= 64 && value.utf8.allSatisfy {
            ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x41 && $0 <= 0x5A) || ($0 >= 0x61 && $0 <= 0x7A) || $0 == 0x2D || $0 == 0x5F
        }
    }

    func learn(_ id: String) throws {
        guard Self.isIdentifier(id) else { throw NotesRejection("badIdentifier", "Apple returned an unexpected resource identifier.") }
        known.insert(id)
    }

    func recordCreated(_ id: String) throws {
        try learn(id)
        created.insert(id)
    }

    func authorize(method: String, path: String) throws {
        if method == "GET", path == Self.builds || path == Self.collection { return }
        if method == "POST", path == Self.collection, mode != .read { return }
        if path.hasPrefix(Self.collection + "/") {
            let id = String(path.dropFirst(Self.collection.count + 1))
            if Self.isIdentifier(id) {
                if method == "PATCH", mode == .write, known.contains(id) { return }
                if method == "DELETE", mode == .probe, created.contains(id) { return }
            }
        }
        throw NotesRejection("forbidden", "\(method) \(path) is outside the notes operation's allow-list for mode \(mode.rawValue).")
    }
}

enum NotesRules {
    /// Sorted, so the probe and the write order are deterministic.
    static let locales = ["en-US", "zh-Hans"]
    /// Apple's limit, counted in UTF-16 code units: the strictest of the plausible ways to count.
    static let maxLength = 4000
    static let maxRequestBytes = 64 * 1024
    /// Two lines with a bullet and CJK text, so reading it back also proves newline and non-ASCII handling.
    static let probeText = "AskHuman helper probe, safe to ignore\n• 探针 probe"

    /// Normalizes line endings, rejects control characters, and strips the trailing run of spaces and newlines.
    static func canonicalize(_ text: String) throws -> String {
        var value = text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
        for scalar in value.unicodeScalars where (scalar.value < 0x20 && scalar.value != 0x0A) || scalar.value == 0x7F {
            throw NotesRejection("control", "Notes may not contain control characters other than newlines.")
        }
        while let last = value.unicodeScalars.last, last == " " || last == "\n" {
            value.unicodeScalars.removeLast()
        }
        guard !value.isEmpty else { throw NotesRejection("empty", "The notes text is empty.") }
        guard value.utf16.count <= maxLength else {
            throw NotesRejection("tooLong", "The notes text is \(value.utf16.count) UTF-16 units; the limit is \(maxLength).")
        }
        return value
    }

    static func validateBuild(_ build: String) throws {
        let scalars = Array(build.unicodeScalars)
        guard (1...9).contains(scalars.count), scalars[0] != "0", scalars.allSatisfy({ $0.value >= 0x30 && $0.value <= 0x39 }) else {
            throw NotesRejection("badBuild", "The build number must be 1-9 ASCII digits without a leading zero.")
        }
    }

    static func parseRequest(_ data: Data) throws -> NotesRequest {
        guard let object = try? JSONSerialization.jsonObject(with: data), let fields = object as? [String: Any],
              Set(fields.keys).isSubset(of: ["mode", "build", "texts"]) else {
            throw NotesRejection("badRequest", "The notes request must be a JSON object with only mode, build and texts.")
        }
        guard let modeName = fields["mode"] as? String, let mode = NotesMode(rawValue: modeName) else {
            throw NotesRejection("badMode", "The mode must be read, write or probe.")
        }
        guard let build = fields["build"] as? String else { throw NotesRejection("badBuild", "The build number must be a string.") }
        try validateBuild(build)
        if mode != .write {
            guard fields["texts"] == nil else { throw NotesRejection("badRequest", "Only write requests carry texts.") }
            return NotesRequest(mode: mode, build: build, texts: [:])
        }
        guard let raw = fields["texts"] as? [String: Any], !raw.isEmpty else {
            throw NotesRejection("badRequest", "A write request needs a non-empty texts object.")
        }
        for locale in raw.keys.sorted() where !locales.contains(locale) {
            throw NotesRejection("badLocale", "Only \(locales.joined(separator: ", ")) are supported.")
        }
        var texts = [String: String]()
        for locale in raw.keys.sorted() {
            guard let text = raw[locale] as? String else { throw NotesRejection("badRequest", "Each text must be a string.") }
            texts[locale] = try canonicalize(text)
        }
        return NotesRequest(mode: mode, build: build, texts: texts)
    }

    /// Reads a request file the launcher wrote: a regular file owned by this user, owner-only, small.
    static func loadRequest(path: String) throws -> NotesRequest {
        let rejection = NotesRejection("badRequestFile", "The notes request must be an owner-only regular file of at most \(maxRequestBytes / 1024) KiB.")
        let descriptor = open(path, O_RDONLY | O_NOFOLLOW)
        guard descriptor >= 0 else { throw rejection }
        defer { close(descriptor) }
        var info = stat()
        guard fstat(descriptor, &info) == 0, (info.st_mode & mode_t(S_IFMT)) == mode_t(S_IFREG), info.st_uid == getuid(),
              info.st_mode & 0o077 == 0, Int(info.st_size) <= maxRequestBytes else { throw rejection }
        let data = FileHandle(fileDescriptor: descriptor, closeOnDealloc: false).readData(ofLength: maxRequestBytes + 1)
        guard data.count <= maxRequestBytes else { throw rejection }
        return try parseRequest(data)
    }

    /// The localizations in a `betaBuildLocalizations` response, by locale.
    static func parseLocalizations(_ body: [String: Any]) throws -> [String: RemoteNote] {
        guard let items = body["data"] as? [[String: Any]] else { throw NotesRejection("badResponse", "Apple returned no localization list.") }
        var notes = [String: RemoteNote]()
        for item in items {
            guard let id = item["id"] as? String, let attributes = item["attributes"] as? [String: Any],
                  let locale = attributes["locale"] as? String else {
                throw NotesRejection("badResponse", "Apple returned a localization without an id or locale.")
            }
            guard notes[locale] == nil else { throw NotesRejection("badResponse", "Apple returned two entries for \(locale).") }
            notes[locale] = RemoteNote(id: id, text: attributes["whatsNew"] as? String ?? "")
        }
        return notes
    }

    /// One step per requested locale: leave an identical text alone, patch a different one, create a missing one.
    static func writePlan(existing: [String: RemoteNote], texts: [String: String]) -> [NotesStep] {
        texts.keys.sorted().map { locale in
            guard let remote = existing[locale] else { return NotesStep(locale: locale, action: .create) }
            return NotesStep(locale: locale, action: remote.text == texts[locale] ? .unchanged : .update(id: remote.id))
        }
    }

    /// The locales a probe may use: those with no entry at all. A probe never touches an existing entry.
    static func probeLocales(existing: Set<String>) throws -> [String] {
        let free = locales.filter { !existing.contains($0) }
        guard !free.isEmpty else {
            throw NotesRejection("probeRefused", "Every supported locale already has an entry; the probe never touches existing notes.")
        }
        return free
    }

    /// A short description of an Apple error response. It never contains headers or credentials.
    static func appleErrorSummary(status: Int, body: Data) -> String {
        var parts = ["Apple rejected the notes request (HTTP \(status))"]
        if let object = try? JSONSerialization.jsonObject(with: body), let errors = (object as? [String: Any])?["errors"] as? [[String: Any]] {
            for error in errors.prefix(3) {
                let fields = ["code", "title", "detail"].compactMap { (error[$0] as? String).map { String($0.prefix(300)) } }
                if !fields.isEmpty { parts.append(fields.joined(separator: " - ")) }
            }
        }
        if status == 401 || status == 403 {
            parts.append("This is an authorization problem: report it to the user and do not raise the API key's privileges")
        }
        return parts.joined(separator: "; ") + "."
    }
}
