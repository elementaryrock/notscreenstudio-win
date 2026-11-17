import AVFoundation
import Foundation
import Testing
import NSSCore
@testable import NSSProject

/// Writes a tiny BGRA movie so thumbnail generation has something real to decode.
private func writeTinyMovie(to url: URL, seconds: Double) async throws {
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 64, AVVideoHeightKey: 64,
    ])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: 64, kCVPixelBufferHeightKey as String: 64,
    ])
    writer.add(input)
    writer.startWriting()
    writer.startSession(atSourceTime: .zero)
    let fps = 10
    for i in 0..<Int(seconds * Double(fps)) {
        var pb: CVPixelBuffer?
        CVPixelBufferCreate(nil, 64, 64, kCVPixelFormatType_32BGRA, nil, &pb)
        guard let pb else { continue }
        CVPixelBufferLockBaseAddress(pb, [])
        memset(CVPixelBufferGetBaseAddress(pb), Int32(40 + i % 200), CVPixelBufferGetDataSize(pb))
        CVPixelBufferUnlockBaseAddress(pb, [])
        while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(2)) }
        adaptor.append(pb, withPresentationTime: CMTime(value: CMTimeValue(i), timescale: CMTimeScale(fps)))
    }
    input.markAsFinished()
    await writer.finishWriting()
    if writer.status != .completed { throw NSSError.io("movie write failed: \(String(describing: writer.error))") }
}

/// Creates a bundle in `library` with a project.json (and optionally a movie).
@discardableResult
private func makeBundle(in library: URL, name: String, created: Date, duration: Double = 3, movie: Bool = false) async throws -> URL {
    let bundle = library.appendingPathComponent("\(name).nssproj", isDirectory: true)
    try FileManager.default.createDirectory(at: bundle, withIntermediateDirectories: true)
    var p = sampleProject(name: name, duration: duration)
    p.createdAt = created
    try FileProjectStore.writeProject(p, to: ProjectReference(url: bundle))
    if movie { try await writeTinyMovie(to: bundle.appendingPathComponent("screen.mov"), seconds: duration) }
    return bundle
}

@Suite struct ProjectLibraryTests {
    @Test func listsNewestFirstAndSkipsJunk() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        try await makeBundle(in: dir, name: "old", created: Date(timeIntervalSince1970: 1000))
        try await makeBundle(in: dir, name: "new", created: Date(timeIntervalSince1970: 3000))
        try await makeBundle(in: dir, name: "mid", created: Date(timeIntervalSince1970: 2000))
        try FileManager.default.createDirectory(at: dir.appendingPathComponent("broken.nssproj"), withIntermediateDirectories: true)
        try Data("x".utf8).write(to: dir.appendingPathComponent("notes.txt"))
        let lib = FileProjectLibrary(libraryURL: dir)
        let names = try await lib.summaries().map(\.name)
        #expect(names == ["new", "mid", "old"])
    }

    @Test func copiedBundlesGetDistinctStableIDs() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let a = try await makeBundle(in: dir, name: "a", created: Date(timeIntervalSince1970: 1))
        let copy = dir.appendingPathComponent("a copy.nssproj")
        try FileManager.default.copyItem(at: a, to: copy)
        let lib = FileProjectLibrary(libraryURL: dir)
        let first = try await lib.summaries()
        let second = try await lib.summaries()
        #expect(first.count == 2 && Set(first.map(\.id)).count == 2)
        #expect(Set(first.map(\.id)) == Set(second.map(\.id)))
    }

    @Test func emptyOrMissingLibraryIsEmpty() async throws {
        let lib = FileProjectLibrary(libraryURL: tempDir())
        #expect(try await lib.summaries().isEmpty)
    }

    @Test func renameChangesNameOnly() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let bundle = try await makeBundle(in: dir, name: "a", created: Date())
        let lib = FileProjectLibrary(libraryURL: dir)
        let s = try #require(try await lib.summaries().first)
        try await lib.rename(s, to: "  Launch demo ")
        let after = try #require(try await lib.summaries().first)
        #expect(after.name == "Launch demo")
        #expect(after.bundle.url.resolvingSymlinksInPath() == bundle.resolvingSymlinksInPath())
        #expect(after.id == s.id)
        await #expect(throws: NSSError.self) { try await lib.rename(s, to: "   ") }
    }

    @Test(arguments: ["{\"schemaVersion\": 99, \"name\": \"x\"}", "not json at all"])
    func renameRefusesUnreadablePrimaryAndKeepsItUntouched(content: String) async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let bundle = try await makeBundle(in: dir, name: "a", created: Date())
        let primary = bundle.appendingPathComponent("project.json")
        // Valid backup, unreadable primary.
        try FileManager.default.copyItem(at: primary, to: bundle.appendingPathComponent("project.json.bak"))
        try Data(content.utf8).write(to: primary)
        let lib = FileProjectLibrary(libraryURL: dir)
        let s = try #require(try await lib.summaries().first)   // listing may use the backup
        await #expect(throws: NSSError.self) { try await lib.rename(s, to: "New") }
        #expect(try Data(contentsOf: primary) == Data(content.utf8))
    }

    @Test func deleteMovesToTrashHook() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let bundle = try await makeBundle(in: dir, name: "a", created: Date())
        let lib = FileProjectLibrary(libraryURL: dir, trash: { try FileManager.default.removeItem(at: $0) })
        let s = try #require(try await lib.summaries().first)
        try await lib.delete(s)
        #expect(!FileManager.default.fileExists(atPath: bundle.path))
        #expect(try await lib.summaries().isEmpty)
    }

    @Test func deleteReportsFailure() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        try await makeBundle(in: dir, name: "a", created: Date())
        struct Boom: Error {}
        let lib = FileProjectLibrary(libraryURL: dir, trash: { _ in throw Boom() })
        let s = try #require(try await lib.summaries().first)
        await #expect(throws: NSSError.self) { try await lib.delete(s) }
    }

    @Test func thumbnailTimeRule() {
        #expect(FileProjectLibrary.thumbnailTime(duration: 10) == 1)
        #expect(FileProjectLibrary.thumbnailTime(duration: 2) == 1)
        #expect(FileProjectLibrary.thumbnailTime(duration: 1) == 0.5)
        #expect(FileProjectLibrary.thumbnailTime(duration: 0) == 0)
    }

    @Test func generatesAndCachesThumbnail() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let bundle = try await makeBundle(in: dir, name: "a", created: Date(), duration: 3, movie: true)
        let lib = FileProjectLibrary(libraryURL: dir)
        let s = try #require(try await lib.summaries().first)
        #expect(s.thumbnailURL == nil)
        let url = try #require(await lib.thumbnailURL(for: s))
        #expect(url.resolvingSymlinksInPath() == bundle.appendingPathComponent("thumbnail.jpg").resolvingSymlinksInPath())
        #expect((try Data(contentsOf: url)).starts(with: [0xFF, 0xD8]))
        // Cached: listing now reports it, and a second request returns the same file without regenerating.
        #expect(try await lib.summaries().first?.thumbnailURL == url)
        let modified = try FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date
        #expect(await lib.thumbnailURL(for: s) == url)
        #expect(try FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date == modified)
    }

    @Test func missingMovieGivesNoThumbnail() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        try await makeBundle(in: dir, name: "a", created: Date())
        let lib = FileProjectLibrary(libraryURL: dir)
        let s = try #require(try await lib.summaries().first)
        #expect(await lib.thumbnailURL(for: s) == nil)
    }
}
