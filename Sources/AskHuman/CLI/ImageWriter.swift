import Foundation

enum ImageWriterError: Error, CustomStringConvertible {
    case decodeFailed(Int)
    case writeFailed(Int, String)

    var description: String {
        switch self {
        case .decodeFailed(let index):
            return "保存图片附件失败 (第 \(index + 1) 张): base64 解码失败"
        case .writeFailed(let index, let msg):
            return "保存图片附件失败 (第 \(index + 1) 张): \(msg)"
        }
    }
}

enum ImageWriter {
    /// 把图片附件写入请求临时目录，返回绝对路径列表
    static func save(_ images: [ImageAttachment], requestId: String) throws -> [String] {
        guard !images.isEmpty else { return [] }
        let dir = Paths.requestTempDir(requestId: requestId)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        var paths: [String] = []
        for (index, img) in images.enumerated() {
            let ext = fileExtension(forMediaType: img.mediaType)
            let filename: String
            if let raw = img.filename?.trimmingCharacters(in: .whitespaces), !raw.isEmpty {
                filename = sanitize(raw, fallbackExt: ext)
            } else {
                filename = "img-\(index + 1).\(ext)"
            }
            guard let bytes = decodeBase64(img.data) else {
                throw ImageWriterError.decodeFailed(index)
            }
            let fileURL = dir.appendingPathComponent(filename, isDirectory: false)
            do {
                try bytes.write(to: fileURL, options: .atomic)
            } catch {
                throw ImageWriterError.writeFailed(index, error.localizedDescription)
            }
            paths.append(fileURL.path)
        }
        return paths
    }

    static func fileExtension(forMediaType mediaType: String) -> String {
        switch mediaType.lowercased() {
        case "image/png": return "png"
        case "image/jpeg", "image/jpg": return "jpg"
        case "image/gif": return "gif"
        case "image/webp": return "webp"
        case "image/bmp": return "bmp"
        case "image/svg+xml": return "svg"
        default: return "bin"
        }
    }

    /// 去掉路径分隔符与危险字符，确保文件落在目标目录内
    static func sanitize(_ raw: String, fallbackExt: String) -> String {
        let base = raw.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? raw
        let forbidden: Set<Character> = ["<", ">", ":", "\"", "|", "?", "*", "\0"]
        let cleaned = String(base.filter { !forbidden.contains($0) })
        let trimmed = cleaned.trimmingCharacters(in: .whitespaces)
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
        if trimmed.isEmpty {
            return "img.\(fallbackExt)"
        }
        return trimmed
    }

    /// 解码 base64，兼容 data:...;base64, 前缀与空白
    static func decodeBase64(_ data: String) -> Data? {
        var payload = data
        if let range = data.range(of: "base64,") {
            payload = String(data[range.upperBound...])
        }
        let cleaned = payload.filter { !$0.isWhitespace }
        return Data(base64Encoded: cleaned)
    }
}
