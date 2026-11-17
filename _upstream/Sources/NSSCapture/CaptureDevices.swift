import AVFoundation
import Foundation

public struct CaptureDeviceInfo: Sendable, Equatable, Identifiable, Hashable {
    /// `AVCaptureDevice.uniqueID`. This is what `RecordingOptions` takes.
    public let id: String
    public let name: String
    public let isDefault: Bool
    public init(id: String, name: String, isDefault: Bool) {
        self.id = id
        self.name = name
        self.isDefault = isDefault
    }
}

/// Lists microphones and cameras. Selection is by `id` in `RecordingOptions`.
public enum CaptureDevices {
    public static func microphones() -> [CaptureDeviceInfo] {
        let discovery = AVCaptureDevice.DiscoverySession(deviceTypes: [.microphone, .external], mediaType: .audio, position: .unspecified)
        return infos(discovery.devices, default: AVCaptureDevice.default(for: .audio))
    }

    public static func cameras() -> [CaptureDeviceInfo] {
        let discovery = AVCaptureDevice.DiscoverySession(
            deviceTypes: [.builtInWideAngleCamera, .external, .continuityCamera, .deskViewCamera],
            mediaType: .video, position: .unspecified)
        return infos(discovery.devices, default: AVCaptureDevice.default(for: .video))
    }

    static func infos(_ devices: [AVCaptureDevice], default def: AVCaptureDevice?) -> [CaptureDeviceInfo] {
        devices.map { CaptureDeviceInfo(id: $0.uniqueID, name: $0.localizedName, isDefault: $0.uniqueID == def?.uniqueID) }
    }

    /// nil when the device is gone.
    static func device(id: String) -> AVCaptureDevice? { AVCaptureDevice(uniqueID: id) }
}
