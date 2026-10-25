import XCTest
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
@testable import AHCore

final class ImageDownscalerTests: XCTestCase {
    private func png(width: Int, height: Int, alpha: Bool = true) throws -> Data {
        let info = alpha ? CGImageAlphaInfo.premultipliedLast.rawValue : CGImageAlphaInfo.noneSkipLast.rawValue
        let context = try XCTUnwrap(CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: info
        ))
        context.setFillColor(CGColor(red: 0.1, green: 0.4, blue: 0.9, alpha: alpha ? 0.5 : 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let image = try XCTUnwrap(context.makeImage())
        let data = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return data as Data
    }

    func testLargeImagesAreScaledToTheEdgeLimitKeepingTheAspectRatio() throws {
        let output = try XCTUnwrap(ImageDownscaler.jpeg(from: try png(width: 4000, height: 2000), maxEdge: 2048))
        XCTAssertEqual(output.width, 2048)
        XCTAssertEqual(output.height, 1024)
        XCTAssertEqual(Array(output.data.prefix(2)), [0xFF, 0xD8], "the result is a JPEG")
    }

    func testSmallImagesAreNotEnlarged() throws {
        let output = try XCTUnwrap(ImageDownscaler.jpeg(from: try png(width: 120, height: 60), maxEdge: 2048))
        XCTAssertEqual(output.width, 120)
        XCTAssertEqual(output.height, 60)
    }

    func testTransparentImagesAreFlattenedAndStayDecodable() throws {
        let output = try XCTUnwrap(ImageDownscaler.jpeg(from: try png(width: 64, height: 64, alpha: true)))
        let source = try XCTUnwrap(CGImageSourceCreateWithData(output.data as CFData, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        XCTAssertEqual(image.width, 64)
        XCTAssertTrue([.none, .noneSkipFirst, .noneSkipLast].contains(image.alphaInfo))
    }

    func testTheResultIsSmallerThanALosslessOriginal() throws {
        let original = try png(width: 3000, height: 3000, alpha: false)
        let output = try XCTUnwrap(ImageDownscaler.jpeg(from: original, maxEdge: 1024))
        XCTAssertLessThan(output.data.count, original.count)
    }

    func testGarbageIsRejected() {
        XCTAssertNil(ImageDownscaler.jpeg(from: Data("not an image".utf8)))
        XCTAssertNil(ImageDownscaler.jpeg(from: Data()))
    }
}
