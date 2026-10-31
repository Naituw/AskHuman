import Foundation

/// A visit to the inbox detail pages. Finished IDs remain as position anchors, never destinations.
public struct RequestNavigation: Equatable, Sendable {
    public let filter: InboxFilter
    public private(set) var ids: [String]

    public init(filter: InboxFilter, ids: [String], current: String) {
        self.filter = filter
        var seen = Set<String>()
        self.ids = ids.filter { seen.insert($0).inserted }
        if !seen.contains(current) { self.ids.insert(current, at: 0) }
    }

    public mutating func append(_ incoming: [String]) {
        var seen = Set(ids)
        ids.append(contentsOf: incoming.filter { seen.insert($0).inserted })
    }

    public func neighbor(of current: String, direction: Int, live: Set<String>) -> String? {
        guard let position = ids.firstIndex(of: current), direction != 0 else { return nil }
        let step = direction < 0 ? -1 : 1
        var index = position + step
        while ids.indices.contains(index) {
            if live.contains(ids[index]) { return ids[index] }
            index += step
        }
        return nil
    }
}

/// A content anchor and its distance above the readable viewport, local to one request visit.
public struct RequestReadPosition: Equatable, Sendable {
    public var anchor: String
    public var distance: Double
    public var fallback: Double

    public init(anchor: String, distance: Double, fallback: Double) {
        self.anchor = anchor
        self.distance = distance
        self.fallback = fallback
    }
}
