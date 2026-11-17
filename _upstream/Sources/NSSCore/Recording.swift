import CoreGraphics
import Foundation

/// What to record.
public enum CaptureSource: Codable, Sendable, Equatable, Hashable {
    /// A whole display (CGDirectDisplayID).
    case display(id: UInt32)
    /// A single window (CGWindowID).
    case window(id: UInt32)
    /// A rectangle on a display. `rect` is in that display's points, origin top-left.
    case area(displayID: UInt32, rect: CGRect)
}

public struct RecordingOptions: Codable, Sendable, Equatable {
    public var source: CaptureSource
    /// AVCaptureDevice uniqueID of the microphone, nil = no mic.
    public var micDeviceID: String?
    /// AVCaptureDevice uniqueID of the camera, nil = no camera.
    public var cameraDeviceID: String?
    public var captureSystemAudio: Bool
    /// Seconds counted down before recording starts; 0 = off.
    public var countdownSeconds: Int

    public init(
        source: CaptureSource,
        micDeviceID: String? = nil,
        cameraDeviceID: String? = nil,
        captureSystemAudio: Bool = false,
        countdownSeconds: Int = 3
    ) {
        self.source = source
        self.micDeviceID = micDeviceID
        self.cameraDeviceID = cameraDeviceID
        self.captureSystemAudio = captureSystemAudio
        self.countdownSeconds = countdownSeconds
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        source = try c.decode(CaptureSource.self, forKey: .source)
        micDeviceID = try c.decodeIfPresent(String.self, forKey: .micDeviceID)
        cameraDeviceID = try c.decodeIfPresent(String.self, forKey: .cameraDeviceID)
        captureSystemAudio = try c.decodeIfPresent(Bool.self, forKey: .captureSystemAudio) ?? false
        countdownSeconds = try c.decodeIfPresent(Int.self, forKey: .countdownSeconds) ?? 3
    }
}

/// Files inside a `.nssproj` bundle. Names are relative to the bundle root.
public struct RecordingAssets: Codable, Sendable, Equatable {
    public var screenFile: String
    public var cameraFile: String?
    public var micFile: String?
    public var cursorFile: String
    /// Pixel size of the recorded screen video.
    public var sourcePixelSize: PixelSize
    /// Frame rate of the screen video.
    public var frameRate: Double
    /// Duration in seconds of the recording.
    public var duration: MediaTime
    /// Shared time origin (t = 0) of the recording: the host-clock time, in seconds on the
    /// `CACurrentMediaTime()` / `CMClockGetHostTimeClock()` clock, of the presentation timestamp
    /// of the FIRST screen frame written to `screenFile`. `RecordingSession.stop()` fills it.
    /// Cursor/click timestamps are rebased to this origin by `InputLogger.stop(recordingStartHostTime:)`.
    public var startHostTime: Double
    /// Project thumbnail (relative to the bundle root); nil until generated.
    public var thumbnailFile: String?

    public static let defaultScreenFile = "screen.mov"
    public static let defaultCameraFile = "camera.mov"
    public static let defaultMicFile = "mic.m4a"
    public static let defaultCursorFile = "cursor.json"

    public init(
        screenFile: String = RecordingAssets.defaultScreenFile,
        cameraFile: String? = nil,
        micFile: String? = nil,
        cursorFile: String = RecordingAssets.defaultCursorFile,
        sourcePixelSize: PixelSize,
        frameRate: Double,
        duration: MediaTime,
        startHostTime: Double = 0,
        thumbnailFile: String? = nil
    ) {
        self.screenFile = screenFile
        self.cameraFile = cameraFile
        self.micFile = micFile
        self.cursorFile = cursorFile
        self.sourcePixelSize = sourcePixelSize
        self.frameRate = frameRate
        self.duration = duration
        self.startHostTime = startHostTime
        self.thumbnailFile = thumbnailFile
    }
}

public enum RecordingState: Codable, Sendable, Equatable {
    case idle
    case preparing
    case recording(startedAt: Date)
    case stopping
    case finished
    case failed(message: String)
}

extension RecordingAssets {
    /// True when `name` is a plain file name that stays inside the project bundle: non-empty, no path
    /// separators, not `.` or `..`, and no NUL.
    public static func isSafeFileName(_ name: String) -> Bool {
        !name.isEmpty && name != "." && name != ".." && !name.contains("/") && !name.contains("\\") && !name.contains("\0")
    }

    /// The first stored file name that is not a plain name inside the bundle, if any.
    public var unsafeFileName: String? {
        ([screenFile, cursorFile] + [cameraFile, micFile, thumbnailFile].compactMap { $0 }).first { !Self.isSafeFileName($0) }
    }
}

// Forward compatibility: fields added after v1 decode with defaults when absent (see PLAN.md).
extension RecordingAssets {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        screenFile = try c.decodeIfPresent(String.self, forKey: .screenFile) ?? Self.defaultScreenFile
        cameraFile = try c.decodeIfPresent(String.self, forKey: .cameraFile)
        micFile = try c.decodeIfPresent(String.self, forKey: .micFile)
        cursorFile = try c.decodeIfPresent(String.self, forKey: .cursorFile) ?? Self.defaultCursorFile
        sourcePixelSize = try c.decode(PixelSize.self, forKey: .sourcePixelSize)
        frameRate = try c.decode(Double.self, forKey: .frameRate)
        duration = try c.decode(MediaTime.self, forKey: .duration)
        startHostTime = try c.decodeIfPresent(Double.self, forKey: .startHostTime) ?? 0
        thumbnailFile = try c.decodeIfPresent(String.self, forKey: .thumbnailFile)
    }
}
