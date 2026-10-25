import Foundation
import UniformTypeIdentifiers
import AHProtocol

/// How an attachment is previewed.
enum AssetKind {
    case image, markdown, diff, text, other

    init(asset: AssetRef) {
        let ext = (asset.name as NSString).pathExtension.lowercased()
        if asset.isImage || asset.mime.hasPrefix("image/") {
            self = .image
        } else if ["md", "markdown", "mdx"].contains(ext) || asset.mime == "text/markdown" {
            self = .markdown
        } else if ["diff", "patch"].contains(ext) || asset.mime == "text/x-diff" {
            self = .diff
        } else if asset.mime.hasPrefix("text/") || Self.textExtensions.contains(ext) || asset.mime == "application/json" {
            self = .text
        } else {
            self = .other
        }
    }

    static let textExtensions: Set<String> = [
        "txt", "log", "json", "yaml", "yml", "toml", "ini", "cfg", "conf", "env", "csv", "tsv", "xml", "html", "htm", "css", "scss",
        "js", "jsx", "ts", "tsx", "mjs", "cjs", "swift", "m", "mm", "h", "c", "cc", "cpp", "hpp", "rs", "go", "py", "rb", "php", "java",
        "kt", "kts", "sh", "zsh", "bash", "fish", "sql", "graphql", "proto", "gradle", "lock", "vue", "svelte", "dockerfile", "makefile",
    ]

    var symbol: String {
        switch self {
        case .image: "photo"
        case .markdown: "doc.richtext"
        case .diff: "plusminus"
        case .text: "doc.text"
        case .other: "doc"
        }
    }
}

extension AssetRef {
    var kind: AssetKind { AssetKind(asset: self) }

    /// The decoded thumbnail the Mac sent along with an image.
    var thumbnailData: Data? { thumb.flatMap(Bytes.b64uDecode) }
}
