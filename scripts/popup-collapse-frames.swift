// Display-frame probe for the collapsible-popup prototype (scripts/popup-collapse-demo.sh --video).
//
// usage: popup-collapse-frames <x> <y> <w> <h> <out.tsv>
//
// Streams a screen region (points, top-left origin, main display) through ScreenCaptureKit and writes one line per
// delivered frame: the display time of the frame (the vsync it was shown at, in ms), the time it reached this process,
// the bounding box of the pixels whose green channel is above 10 (the prototype puts a magenta backdrop behind the
// answer window, so "anything with green" is the window) and a content hash. A video encoder is deliberately not
// involved: its own frame pacing would be mixed into the measurement.
//
// Needs the screen recording permission of the calling terminal. Stops on SIGINT / SIGTERM.
import CoreImage
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

setvbuf(stdout, nil, _IOLBF, 0)
let arguments = CommandLine.arguments
guard arguments.count >= 6, let regionX = Double(arguments[1]), let regionY = Double(arguments[2]),
      let regionW = Double(arguments[3]), let regionH = Double(arguments[4]) else {
    FileHandle.standardError.write(Data("usage: popup-collapse-frames x y w h out.tsv\n".utf8))
    exit(2)
}
FileManager.default.createFile(atPath: arguments[5], contents: nil)
guard let outputHandle = FileHandle(forWritingAtPath: arguments[5]) else { exit(2) }

var timebase = mach_timebase_info_data_t()
mach_timebase_info(&timebase)
func milliseconds(_ ticks: UInt64) -> Double {
    Double(ticks) * Double(timebase.numer) / Double(timebase.denom) / 1_000_000
}

final class Probe: NSObject, SCStreamOutput, SCStreamDelegate {
    private let output: FileHandle
    private var origin: Double?
    private var buffer = ""
    private var delivered = 0
    // POPUP_FRAMES_DUMP=<directory>: also write a few frames as PNG (to see what the probe sees).
    private let dumpDirectory = ProcessInfo.processInfo.environment["POPUP_FRAMES_DUMP"]
    private let context = CIContext()

    init(output: FileHandle) {
        self.output = output
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sample: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sample.isValid,
              let attachments = (CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false)
                  as? [[SCStreamFrameInfo: Any]])?.first,
              let rawStatus = attachments[SCStreamFrameInfo.status] as? Int,
              SCFrameStatus(rawValue: rawStatus) == .complete,
              let pixels = sample.imageBuffer else { return }
        let received = milliseconds(mach_absolute_time())
        let display = (attachments[SCStreamFrameInfo.displayTime] as? UInt64).map(milliseconds) ?? received
        if origin == nil {
            origin = display
            // Times in the lines below are relative to the first frame; this line anchors them to the mach clock.
            buffer += String(format: "# origin %.3f\n", display)
        }
        delivered += 1
        if let directory = dumpDirectory, [1, 100, 400, 1500].contains(delivered) {
            try? FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
            try? context.writePNGRepresentation(of: CIImage(cvPixelBuffer: pixels),
                                                to: URL(fileURLWithPath: "\(directory)/frame-\(delivered).png"),
                                                format: .RGBA8, colorSpace: CGColorSpace(name: CGColorSpace.sRGB)!)
        }

        CVPixelBufferLockBaseAddress(pixels, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(pixels)?.assumingMemoryBound(to: UInt8.self) else { return }
        let width = CVPixelBufferGetWidth(pixels), height = CVPixelBufferGetHeight(pixels)
        let stride = CVPixelBufferGetBytesPerRow(pixels)

        var minX = Int.max, maxX = -1, minY = Int.max, maxY = -1
        var hash: UInt64 = 1469598103934665603
        // The stream blends the outermost pixels with what lies beyond the region: stay a few pixels inside.
        let inset = 4
        for row in inset..<(height - inset) {
            let line = base + row * stride
            var left = inset
            while left < width - inset && line[left * 4 + 1] <= 10 { left += 1 }
            if left == width - inset { continue }
            var right = width - 1 - inset
            while right > left && line[right * 4 + 1] <= 10 { right -= 1 }
            minX = min(minX, left); maxX = max(maxX, right)
            if minY == Int.max { minY = row }
            maxY = row
            if row % 4 == 0 {
                var column = left
                while column <= right {
                    hash = (hash ^ UInt64(line[column * 4 + 1])) &* 1099511628211
                    column += 8
                }
            }
        }
        if maxX < 0 { minX = 0; maxX = -1; minY = 0; maxY = -1 }
        buffer += String(format: "%.3f\t%.3f\t%d\t%d\t%d\t%d\t%llu\n", display - (origin ?? display), received - (origin ?? display),
                         minX, minY, maxX, maxY, hash)
        if buffer.utf8.count > 16_384 { flush() }
    }

    func flush() {
        if !buffer.isEmpty { output.write(Data(buffer.utf8)); buffer = "" }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        FileHandle.standardError.write(Data("stream stopped: \(error)\n".utf8))
    }
}

let probe = Probe(output: outputHandle)
let queue = DispatchQueue(label: "popup-collapse-frames", qos: .userInteractive)
var activeStream: SCStream?

Task {
    do {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let display = content.displays.first(where: { $0.displayID == CGMainDisplayID() }) ?? content.displays.first else {
            FileHandle.standardError.write(Data("no display\n".utf8))
            exit(1)
        }
        let filter = SCContentFilter(display: display, excludingApplications: [], exceptingWindows: [])
        let configuration = SCStreamConfiguration()
        configuration.sourceRect = CGRect(x: regionX, y: regionY, width: regionW, height: regionH)
        configuration.width = Int(regionW)
        configuration.height = Int(regionH)
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: 240)
        configuration.queueDepth = 8
        configuration.showsCursor = false
        configuration.pixelFormat = kCVPixelFormatType_32BGRA
        let stream = SCStream(filter: filter, configuration: configuration, delegate: probe)
        try stream.addStreamOutput(probe, type: .screen, sampleHandlerQueue: queue)
        try await stream.startCapture()
        activeStream = stream
        print("ready")
    } catch {
        FileHandle.standardError.write(Data("capture failed: \(error)\n".utf8))
        exit(1)
    }
}

func stop() {
    Task {
        try? await activeStream?.stopCapture()
        queue.sync { probe.flush() }
        exit(0)
    }
}
signal(SIGINT, SIG_IGN)
signal(SIGTERM, SIG_IGN)
let interrupt = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
interrupt.setEventHandler(handler: stop)
interrupt.resume()
let terminate = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
terminate.setEventHandler(handler: stop)
terminate.resume()
RunLoop.main.run()
