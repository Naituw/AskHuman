import Foundation

/// The names of the files of one draft. No two of them are the same: the list shows them one under the other and
/// the Agent finds them side by side. (A repeated name would not overwrite anything, since every file is stored under
/// an id of its own, but the files could not be told apart.)
enum AttachmentNames {
    /// `name` as it is, or with `-2`, `-3`, … before its extension until no name in `taken` is the same. Letter case
    /// is ignored: the disk of the Mac does not tell `Photo.jpg` from `photo.jpg` either.
    static func unique(_ name: String, among taken: [String]) -> String {
        let used = Set(taken.map { $0.lowercased() })
        guard used.contains(name.lowercased()) else { return name }
        let path = name as NSString
        let stem = path.deletingPathExtension
        let ext = path.pathExtension
        var number = 2
        while true {
            let candidate = ext.isEmpty ? "\(stem)-\(number)" : "\(stem)-\(number).\(ext)"
            if !used.contains(candidate.lowercased()) { return candidate }
            number += 1
        }
    }
}
