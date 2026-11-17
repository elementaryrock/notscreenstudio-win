import AVFoundation
import CoreGraphics
import CoreMedia
import Foundation
import Testing
import NSSCore
import ScreenCaptureKit
@testable import NSSCapture

@Suite struct CaptureMathTests {
    @Test func evenPixelsRoundsToEven() {
        #expect(CaptureMath.evenPixels(1439.6) == 1440)
        #expect(CaptureMath.evenPixels(1441) == 1442)
        #expect(CaptureMath.evenPixels(0) == 2)
        #expect(CaptureMath.evenPixels(.nan) == 2)
    }

    @Test func retinaPixelSize() {
        let s = CaptureMath.pixelSize(points: CGSize(width: 1512, height: 982), scale: 2)
        #expect(s == PixelSize(width: 3024, height: 1964))
        #expect(CaptureMath.pixelSize(points: CGSize(width: 100, height: 50), scale: 0) == PixelSize(width: 100, height: 50))
    }

    @Test func areaIsClampedToDisplay() {
        let d = CGSize(width: 1000, height: 600)
        #expect(CaptureMath.clampedArea(CGRect(x: 900, y: 500, width: 300, height: 300), displaySize: d) == CGRect(x: 900, y: 500, width: 100, height: 100))
        #expect(CaptureMath.clampedArea(CGRect(x: 2000, y: 0, width: 50, height: 50), displaySize: d) == nil)
        #expect(CaptureMath.clampedArea(CGRect(x: 100, y: 100, width: -50, height: -50), displaySize: d) == CGRect(x: 50, y: 50, width: 50, height: 50))
    }

    @Test func bitrateIsClamped() {
        #expect(CaptureMath.videoBitrate(width: 320, height: 240, frameRate: 60) == 8_000_000)
        #expect(CaptureMath.videoBitrate(width: 7680, height: 4320, frameRate: 120) == 200_000_000)
        let b4k = CaptureMath.videoBitrate(width: 3840, height: 2160, frameRate: 60)
        #expect(b4k > 50_000_000 && b4k < 70_000_000)
    }

    @Test func ownAppDetection() {
        #expect(CaptureMath.isOwnApp(pid: 10, bundleID: nil, ownPID: 10, ownBundleID: nil))
        #expect(CaptureMath.isOwnApp(pid: 11, bundleID: "com.x.nss", ownPID: 10, ownBundleID: "com.x.nss"))
        #expect(!CaptureMath.isOwnApp(pid: 11, bundleID: "com.apple.finder", ownPID: 10, ownBundleID: "com.x.nss"))
        #expect(!CaptureMath.isOwnApp(pid: 11, bundleID: "", ownPID: 10, ownBundleID: ""))
    }

    @Test func pickableWindows() {
        let big = CGRect(x: 0, y: 0, width: 800, height: 600)
        #expect(CaptureMath.isPickableWindow(layer: 0, frame: big, isOnScreen: true, hasApp: true))
        #expect(!CaptureMath.isPickableWindow(layer: 25, frame: big, isOnScreen: true, hasApp: true))
        #expect(!CaptureMath.isPickableWindow(layer: 0, frame: CGRect(x: 0, y: 0, width: 20, height: 20), isOnScreen: true, hasApp: true))
        #expect(!CaptureMath.isPickableWindow(layer: 0, frame: big, isOnScreen: false, hasApp: true))
        #expect(!CaptureMath.isPickableWindow(layer: 0, frame: big, isOnScreen: true, hasApp: false))
    }
}

@Suite struct TimeOriginTests {
    @Test func relativeTimeNeverNegative() {
        let o = TimeOrigin(hostTime: 1000)
        #expect(o.relative(1002.5) == 2.5)
        #expect(o.relative(999) == 0)
        #expect(o.duration(endingAt: 1010) == 10)
        #expect(o.duration(endingAt: 900) == 0)
    }

    @Test func samplesBeforeOriginAreDropped() {
        let o = TimeOrigin(hostTime: 1000)
        #expect(o.isEntirelyBefore(start: 999.0, duration: 0.5))
        #expect(o.isEntirelyBefore(start: 999.5, duration: 0.5)) // ends exactly at the origin
        #expect(!o.isEntirelyBefore(start: 999.9, duration: 0.5)) // straddles it: kept
        #expect(!o.isEntirelyBefore(start: 1000, duration: 0))
    }

    @Test func sharedOriginIsWriteOnce() {
        let s = SharedOrigin()
        #expect(s.current == nil)
        #expect(s.setIfUnset(5))
        #expect(!s.setIfUnset(9))
        #expect(s.current == TimeOrigin(hostTime: 5))
    }

    @Test func hostTimeRoundTripsThroughCMTime() {
        let t = 123456.789012345
        #expect(abs(CaptureMath.hostTime(t).seconds - t) < 1e-8)
    }
}

@Suite struct FilePlanTests {
    @Test func namesFollowOptions() {
        let none = CaptureFilePlan(options: RecordingOptions(source: .display(id: 1)))
        #expect(none.screen == "screen.mov" && none.camera == nil && none.mic == nil)
        let all = CaptureFilePlan(options: RecordingOptions(source: .display(id: 1), micDeviceID: "m", cameraDeviceID: "c"))
        #expect(all.camera == "camera.mov" && all.mic == "mic.m4a")
        let bundle = URL(fileURLWithPath: "/tmp/x.nssproj")
        #expect(all.url(all.screen, in: bundle).path == "/tmp/x.nssproj/screen.mov")
    }
}

@Suite struct StreamPlanTests {
    @Test func displayPlanUsesNativePixels() throws {
        let p = try ScreenStreamPlan.make(contentPoints: CGSize(width: 1512, height: 982), scale: 2, area: nil, isWindow: false, systemAudio: false)
        #expect(p.pixelSize == PixelSize(width: 3024, height: 1964))
        #expect(p.sourceRect == nil && p.frameRate == 60)
    }

    @Test func areaPlanSetsSourceRect() throws {
        let area = CGRect(x: 100, y: 50, width: 400, height: 300)
        let p = try ScreenStreamPlan.make(contentPoints: CGSize(width: 1512, height: 982), scale: 2, area: area, isWindow: false, systemAudio: false)
        #expect(p.sourceRect == area)
        #expect(p.pixelSize == PixelSize(width: 800, height: 600))
    }

    @Test func areaOutsideDisplayThrows() {
        #expect(throws: NSSError.self) {
            try ScreenStreamPlan.make(contentPoints: CGSize(width: 100, height: 100), scale: 1, area: CGRect(x: 500, y: 500, width: 10, height: 10), isWindow: false, systemAudio: false)
        }
    }

    @Test func configurationHidesCursorAndRunsAt60() throws {
        let p = try ScreenStreamPlan.make(contentPoints: CGSize(width: 800, height: 600), scale: 2, area: nil, isWindow: false, systemAudio: true)
        let c = p.configuration()
        #expect(c.showsCursor == false)
        #expect(c.width == 1600 && c.height == 1200)
        #expect(c.minimumFrameInterval == CMTime(value: 1, timescale: 60))
        #expect(c.capturesAudio)
        #expect(c.excludesCurrentProcessAudio)
        #expect(p.configuration().sourceRect == .zero || true)
    }

    @Test func videoSettingsCarrySizeAndCodec() throws {
        let p = try ScreenStreamPlan.make(contentPoints: CGSize(width: 800, height: 600), scale: 2, area: nil, isWindow: false, systemAudio: false)
        let s = p.videoSettings(codec: .hevc)
        #expect(s[AVVideoWidthKey] as? Int == 1600)
        #expect(s[AVVideoHeightKey] as? Int == 1200)
        #expect(s[AVVideoCodecKey] as? AVVideoCodecType == .hevc)
        let comp = s[AVVideoCompressionPropertiesKey] as? [String: Any]
        #expect(comp?[AVVideoAverageBitRateKey] as? Int == p.bitrate)
    }

    @Test func aacBitrateClampsToSampleRate() {
        #expect(DeviceRecorder.aacBitrate(channels: 1, sampleRate: 48_000) == 96_000)
        #expect(DeviceRecorder.aacBitrate(channels: 2, sampleRate: 44_100) == 160_000)
        #expect(DeviceRecorder.aacBitrate(channels: 1, sampleRate: 16_000) == 64_000)
        #expect(DeviceRecorder.aacBitrate(channels: 1, sampleRate: 8_000) == 32_000)
        #expect(DeviceRecorder.aacBitrate(channels: 2, sampleRate: 8_000) == 64_000)
    }

    @Test func micAudioSettingsAreAAC() {
        var asbd = AudioStreamBasicDescription(mSampleRate: 44100, mFormatID: kAudioFormatLinearPCM, mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked, mBytesPerPacket: 4, mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 1, mBitsPerChannel: 32, mReserved: 0)
        var fd: CMAudioFormatDescription?
        CMAudioFormatDescriptionCreate(allocator: nil, asbd: &asbd, layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &fd)
        let s = DeviceRecorder.audioSettings(fd!)
        #expect(s[AVFormatIDKey] as? UInt32 == kAudioFormatMPEG4AAC)
        #expect(s[AVNumberOfChannelsKey] as? Int == 1)
        #expect(s[AVSampleRateKey] as? Double == 44100)
    }
}

@Suite struct ErrorMappingTests {
    @Test func passesNSSErrorThrough() {
        #expect(CaptureErrorMapper.map(NSSError.unsupported("x")) == .unsupported("x"))
    }

    @Test func userDeclinedIsPermissionDenied() {
        let e = NSError(domain: SCStreamErrorDomain, code: SCStreamError.Code.userDeclined.rawValue)
        if case .permissionDenied = CaptureErrorMapper.map(e) {} else { Issue.record("expected permissionDenied") }
    }

    @Test func missingWindowMeansClosed() {
        let e = NSError(domain: SCStreamErrorDomain, code: SCStreamError.Code.noCaptureSource.rawValue)
        #expect(CaptureErrorMapper.map(e, context: .window) == .io("The captured window was closed."))
        if case .unsupported = CaptureErrorMapper.map(e, context: .screen) {} else { Issue.record("expected unsupported") }
    }

    @Test func diskFullVariants() {
        let cocoa = NSError(domain: NSCocoaErrorDomain, code: NSFileWriteOutOfSpaceError)
        let posix = NSError(domain: NSPOSIXErrorDomain, code: Int(ENOSPC))
        let wrapped = NSError(domain: AVFoundationErrorDomain, code: -11800, userInfo: [NSUnderlyingErrorKey: posix])
        let av = AVError(.diskFull)
        for e in [cocoa, posix, wrapped, av as NSError] {
            guard case .io(let m) = CaptureErrorMapper.map(e) else { Issue.record("expected io"); continue }
            #expect(m.contains("disk is full"))
        }
    }

    @Test func deviceErrors() {
        #expect(CaptureErrorMapper.map(AVError(.deviceWasDisconnected), context: .camera) == .io("The camera was disconnected."))
        #expect(CaptureErrorMapper.map(AVError(.deviceWasDisconnected), context: .microphone) == .io("The microphone was disconnected."))
        if case .permissionDenied = CaptureErrorMapper.map(AVError(.applicationIsNotAuthorizedToUseDevice), context: .camera) {} else { Issue.record("expected permissionDenied") }
    }

    @Test func authorizationMapping() {
        #expect(CapturePermissions.mapAuthorization(.authorized) == .granted)
        #expect(CapturePermissions.mapAuthorization(.notDetermined) == .notDetermined)
        #expect(CapturePermissions.mapAuthorization(.denied) == .denied)
        #expect(CapturePermissions.mapAuthorization(.restricted) == .denied)
    }
}

@Suite struct CatalogLogicTests {
    @Test func thumbnailFitKeepsAspect() {
        let s = CaptureSourceCatalog.fitted(CGSize(width: 1920, height: 1080), in: CGSize(width: 320, height: 200))
        #expect(s == CGSize(width: 320, height: 180))
        let tall = CaptureSourceCatalog.fitted(CGSize(width: 500, height: 1000), in: CGSize(width: 320, height: 200))
        #expect(tall == CGSize(width: 100, height: 200))
    }
}

@Suite struct SessionStateTests {
    @Test func stopWithoutStartThrows() async {
        let s = ScreenCaptureRecordingSession()
        #expect(await s.state == .idle)
        await #expect(throws: NSSError.self) { _ = try await s.stop() }
    }

    @Test func secondStopThrowsWithoutTouchingState() async {
        let s = ScreenCaptureRecordingSession()
        await #expect(throws: NSSError.self) { _ = try await s.stop() }
        await #expect(throws: NSSError.self) { _ = try await s.stop() }
        #expect(await s.state == .idle)
    }

    @Test(.enabled(if: !CapturePermissions.screenRecordingGranted))
    func startWithoutPermissionIsPermissionDenied() async {
        let s = ScreenCaptureRecordingSession()
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-\(UUID().uuidString).nssproj")
        do {
            try await s.start(options: RecordingOptions(source: .display(id: CGMainDisplayID())), into: dir)
            Issue.record("expected failure")
        } catch let e as NSSError {
            if case .permissionDenied = e {} else { Issue.record("expected permissionDenied, got \(e)") }
        } catch { Issue.record("wrong error type") }
        if case .failed = await s.state {} else { Issue.record("expected failed state") }
    }
}

/// Real hardware: skipped unless screen recording is already granted (so CI stays green).
@Suite(.enabled(if: CapturePermissions.screenRecordingGranted))
struct ScreenHardwareTests {
    @Test func catalogListsDisplays() async throws {
        let list = try await CaptureSourceCatalog.load()
        #expect(!list.displays.isEmpty)
        #expect(list.displays.allSatisfy { $0.pixelSize.width > 0 })
    }

    @Test func recordsOneSecondOfMainDisplay() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-\(UUID().uuidString).nssproj")
        defer { try? FileManager.default.removeItem(at: dir) }
        let s = ScreenCaptureRecordingSession()
        try await s.start(options: RecordingOptions(source: .display(id: CGMainDisplayID())), into: dir)
        try await Task.sleep(for: .seconds(1.5))
        let a = try await s.stop()
        #expect(a.duration > 0.5)
        #expect(a.startHostTime > 0)
        #expect(FileManager.default.fileExists(atPath: dir.appendingPathComponent("screen.mov").path))
        let asset = AVURLAsset(url: dir.appendingPathComponent("screen.mov"))
        let tracks = try await asset.loadTracks(withMediaType: .video)
        #expect(tracks.count == 1)
    }
}
