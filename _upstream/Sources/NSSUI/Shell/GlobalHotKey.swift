import AppKit
import Carbon.HIToolbox

/// The key that stops a recording from anywhere: Control + Command + R.
public enum StopShortcut {
    public static let display = "⌃⌘R"
    static let keyCode = UInt32(kVK_ANSI_R)
    static let modifiers = UInt32(controlKey | cmdKey)
}

private final class HotKeyBox: @unchecked Sendable {
    let handler: @MainActor () -> Void
    init(_ handler: @escaping @MainActor () -> Void) { self.handler = handler }
}

/// One system-wide hot key (Carbon `RegisterEventHotKey`). Needs no Accessibility or Input Monitoring permission.
@MainActor
final class GlobalHotKey {
    private var hotKeyRef: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?
    private var box: HotKeyBox?

    @discardableResult
    func register(keyCode: UInt32, modifiers: UInt32, handler: @escaping @MainActor () -> Void) -> Bool {
        unregister()
        let box = HotKeyBox(handler)
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let callback: EventHandlerUPP = { _, _, userData in
            guard let userData else { return noErr }
            let box = Unmanaged<HotKeyBox>.fromOpaque(userData).takeUnretainedValue()
            // Hot key events arrive on the main thread.
            MainActor.assumeIsolated { box.handler() }
            return noErr
        }
        let userData = Unmanaged.passUnretained(box).toOpaque()
        guard InstallEventHandler(GetApplicationEventTarget(), callback, 1, &spec, userData, &handlerRef) == noErr else {
            return false
        }
        let id = EventHotKeyID(signature: OSType(0x4E53_5354), id: 1) // 'NSST'
        guard RegisterEventHotKey(keyCode, modifiers, id, GetApplicationEventTarget(), 0, &hotKeyRef) == noErr else {
            if let handlerRef { RemoveEventHandler(handlerRef) }
            handlerRef = nil
            return false
        }
        self.box = box
        return true
    }

    func unregister() {
        if let hotKeyRef { UnregisterEventHotKey(hotKeyRef) }
        if let handlerRef { RemoveEventHandler(handlerRef) }
        hotKeyRef = nil
        handlerRef = nil
        box = nil
    }
}
