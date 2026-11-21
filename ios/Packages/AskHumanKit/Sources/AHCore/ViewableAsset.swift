import Foundation

/// A picture ready to be looked at (`RemoteBackend.downloadForViewing`): where it is on the phone, and whether it is the
/// downscaled preview the Mac made of it or the file itself (docs/specs/ios-attachment-traffic.md §7.1).
public struct ViewableAsset: Equatable, Sendable {
    public var url: URL
    /// `true` when `url` is a preview: the viewer then offers the file ("original"), and sharing fetches that first.
    public var isPreview: Bool

    public init(url: URL, isPreview: Bool) {
        self.url = url
        self.isPreview = isPreview
    }
}
