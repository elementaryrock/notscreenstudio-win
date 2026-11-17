import Darwin
import Foundation
import NSSCore

/// Crash-safe file replacement: the data goes to a temp file in the same directory, is flushed to disk,
/// then `rename(2)` swaps it over the destination. A crash leaves either the old file or the new one, never half of one.
public enum AtomicFile {
    public static let tempMarker = ".nss-tmp-"

    /// - Parameter beforeReplace: test hook that runs after the temp file is fully written and before the swap.
    public static func write(_ data: Data, to url: URL, beforeReplace: (() throws -> Void)? = nil) throws {
        let dir = url.deletingLastPathComponent()
        let temp = dir.appendingPathComponent(".\(url.lastPathComponent)\(tempMarker)\(UUID().uuidString)")
        do {
            guard FileManager.default.createFile(atPath: temp.path, contents: nil) else {
                throw NSSError.io("Cannot create \(temp.lastPathComponent)")
            }
            let handle = try FileHandle(forWritingTo: temp)
            do {
                try handle.write(contentsOf: data)
                try handle.synchronize()
                try handle.close()
            } catch {
                try? handle.close()
                throw error
            }
            try beforeReplace?()
            guard rename(temp.path, url.path) == 0 else {
                throw NSSError.io("Cannot replace \(url.lastPathComponent): \(String(cString: strerror(errno)))")
            }
        } catch {
            try? FileManager.default.removeItem(at: temp)
            throw error
        }
    }

    /// Removes temp files a crash left behind in `directory`.
    @discardableResult
    public static func removeStaleTemps(in directory: URL) -> Int {
        let items = (try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []
        var removed = 0
        for name in items where name.hasPrefix(".") && name.contains(tempMarker) {
            if (try? FileManager.default.removeItem(at: directory.appendingPathComponent(name))) != nil { removed += 1 }
        }
        return removed
    }
}
