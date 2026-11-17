import Foundation
import Testing
import NSSCore
import os
@testable import NSSProject

func tempDir() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent("nss-test-\(UUID().uuidString)")
}

func sampleProject(name: String = "t", duration: Double = 1) -> Project {
    Project(name: name, assets: RecordingAssets(sourcePixelSize: PixelSize(width: 100, height: 100), frameRate: 60, duration: duration))
}

/// Deterministic clock: sleepers wake only when `advance` passes their deadline.
final class ManualClock: DebounceClock, @unchecked Sendable {
    private struct Waiter { var id: Int; var deadline: Double; var cont: CheckedContinuation<Void, Error> }
    private struct S { var now = 0.0; var nextID = 0; var waiters: [Waiter] = [] }
    private let state = OSAllocatedUnfairLock(initialState: S())

    var sleeperCount: Int { state.withLock { $0.waiters.count } }

    func sleep(for seconds: Double) async throws {
        let id = state.withLock { s -> Int in s.nextID += 1; return s.nextID }
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
                let cancelled = Task.isCancelled
                state.withLock { s in
                    if cancelled { c.resume(throwing: CancellationError()); return }
                    s.waiters.append(Waiter(id: id, deadline: s.now + seconds, cont: c))
                }
            }
        } onCancel: {
            let w = state.withLock { s -> Waiter? in
                guard let i = s.waiters.firstIndex(where: { $0.id == id }) else { return nil }
                return s.waiters.remove(at: i)
            }
            w?.cont.resume(throwing: CancellationError())
        }
    }

    func advance(by seconds: Double) {
        let due = state.withLock { s -> [Waiter] in
            s.now += seconds
            let due = s.waiters.filter { $0.deadline <= s.now }
            s.waiters.removeAll { $0.deadline <= s.now }
            return due
        }
        for w in due { w.cont.resume() }
    }

    /// Yields until `n` sleepers are registered (bounded, no wall-clock sleep).
    func waitForSleepers(_ n: Int) async {
        for _ in 0..<10_000 where sleeperCount != n { await Task.yield() }
    }
}

/// Collects writes made by a debouncer.
final class WriteLog: @unchecked Sendable {
    private let lock = OSAllocatedUnfairLock(initialState: [(String, URL)]())
    func add(_ p: Project, _ r: ProjectReference) { lock.withLock { $0.append((p.name, r.url)) } }
    var names: [String] { lock.withLock { $0.map(\.0) } }
}

/// Spins on `Task.yield` until `check` holds; bounded, never sleeps.
func settle(_ check: () -> Bool) async {
    for _ in 0..<10_000 where !check() { await Task.yield() }
}

@Suite struct NSSProjectTests {
    @Test func saveAndLoadRoundTrip() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let ref = try await store.createProject(named: "t")
        let project = sampleProject()
        try await store.save(project, to: ref)
        let loaded = try await store.load(ref)
        #expect(loaded == project)
        let track = CursorTrack(samples: [CursorSample(t: 0, x: 0.1, y: 0.2)])
        try await store.saveCursorTrack(track, for: ref, assets: project.assets)
        #expect(try await store.loadCursorTrack(for: ref, assets: project.assets) == track)
        #expect(try await store.listProjects().count == 1)
    }

    @Test func sameNameGivesDistinctSafeBundles() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let a = try await store.createProject(named: "Demo")
        let b = try await store.createProject(named: "Demo")
        #expect(a != b)
        let evil = try await store.createProject(named: "../../escape/x")
        #expect(evil.url.deletingLastPathComponent().standardizedFileURL == dir.standardizedFileURL)
        #expect(try await store.listProjects().count == 3)
    }

    @Test func cursorTrackRoundTripsFullAndIsCompact() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let ref = try await store.createProject(named: "c")
        let assets = sampleProject().assets
        let track = CursorTrack(
            samples: (0..<1000).map { CursorSample(t: Double($0) / 120, x: Double($0) / 1000, y: 0.5) },
            clicks: [ClickEvent(t: 1, x: 0.5, y: 0.5, button: .right, kind: .up)])
        try await store.saveCursorTrack(track, for: ref, assets: assets)
        #expect(try await store.loadCursorTrack(for: ref, assets: assets) == track)
        let text = try String(contentsOf: ref.url.appendingPathComponent("cursor.json"), encoding: .utf8)
        #expect(!text.contains("\n"))
    }

    @Test func missingCursorFileGivesEmptyTrack() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let ref = try await store.createProject(named: "m")
        #expect(try await store.loadCursorTrack(for: ref, assets: sampleProject().assets) == .empty)
    }

    // MARK: Missing / corrupt

    @Test func missingProjectFileThrowsInvalidProject() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let ref = try await store.createProject(named: "x")
        await #expect(throws: NSSError.self) { try await store.load(ref) }
        await #expect(throws: NSSError.self) { try await store.load(ProjectReference(url: dir.appendingPathComponent("nope.nssproj"))) }
    }

    @Test func corruptProjectFallsBackToBackupElseThrows() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let ref = try await store.createProject(named: "x")
        let v1 = sampleProject(name: "one")
        let v2 = sampleProject(name: "two")
        try await store.save(v1, to: ref)
        try await store.save(v2, to: ref)  // v1 becomes project.json.bak
        let file = ref.url.appendingPathComponent("project.json")
        try Data("{ truncated".utf8).write(to: file)
        #expect(try await store.load(ref).name == "one")
        try? FileManager.default.removeItem(at: ref.url.appendingPathComponent("project.json.bak"))
        await #expect(throws: NSSError.self) { try await store.load(ref) }
    }

    // MARK: Atomic write

    @Test func atomicWriteReplacesAndLeavesNoTemps() throws {
        let dir = tempDir()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let f = dir.appendingPathComponent("a.json")
        try AtomicFile.write(Data("one".utf8), to: f)
        try AtomicFile.write(Data("two".utf8), to: f)
        #expect(try String(contentsOf: f, encoding: .utf8) == "two")
        #expect(try FileManager.default.contentsOfDirectory(atPath: dir.path) == ["a.json"])
    }

    @Test func failedAtomicWriteKeepsOldFileAndCleansUp() throws {
        let dir = tempDir()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let f = dir.appendingPathComponent("a.json")
        try AtomicFile.write(Data("good".utf8), to: f)
        struct Boom: Error {}
        #expect(throws: Boom.self) {
            try AtomicFile.write(Data("bad".utf8), to: f, beforeReplace: { throw Boom() })
        }
        #expect(try String(contentsOf: f, encoding: .utf8) == "good")
        #expect(try FileManager.default.contentsOfDirectory(atPath: dir.path) == ["a.json"])
    }

    @Test func staleTempsFromACrashAreRemoved() throws {
        let dir = tempDir()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        try Data("junk".utf8).write(to: dir.appendingPathComponent(".project.json\(AtomicFile.tempMarker)ABC"))
        try Data("keep".utf8).write(to: dir.appendingPathComponent("project.json"))
        #expect(AtomicFile.removeStaleTemps(in: dir) == 1)
        #expect(try FileManager.default.contentsOfDirectory(atPath: dir.path) == ["project.json"])
    }

    // MARK: Migration

    @Test func migratorRunsStepsInOrderAndStampsVersion() throws {
        let m = ProjectMigrator(currentVersion: 3, steps: [
            1: { json in json["name"] = (json["name"] as? String ?? "") + "+1" },
            2: { json in json["name"] = (json["name"] as? String ?? "") + "+2" },
        ])
        var old = try JSONSerialization.jsonObject(with: JSONEncoder().encode(sampleProject(name: "p"))) as! [String: Any]
        old["schemaVersion"] = 1
        let out = try m.migrate(JSONSerialization.data(withJSONObject: old))
        let json = try JSONSerialization.jsonObject(with: out) as! [String: Any]
        #expect(json["name"] as? String == "p+1+2")
        #expect(json["schemaVersion"] as? Int == 3)
    }

    @Test func migratorTreatsMissingVersionAsV1AndRejectsNewerOrUnknown() throws {
        let current = ProjectMigrator.standard
        var obj = try JSONSerialization.jsonObject(with: JSONEncoder().encode(sampleProject())) as! [String: Any]
        obj.removeValue(forKey: "schemaVersion")
        let data = try JSONSerialization.data(withJSONObject: obj)
        let migrated = try JSONSerialization.jsonObject(with: current.migrate(data)) as! [String: Any]
        #expect(migrated["schemaVersion"] as? Int == Project.currentSchemaVersion, "a missing version is v1 and is migrated")
        obj["schemaVersion"] = Project.currentSchemaVersion
        let currentData = try JSONSerialization.data(withJSONObject: obj)
        #expect(try current.migrate(currentData) == currentData)

        obj["schemaVersion"] = Project.currentSchemaVersion + 1
        #expect(throws: ProjectMigrationError.newerSchema(found: Project.currentSchemaVersion + 1, supported: Project.currentSchemaVersion)) {
            try current.migrate(JSONSerialization.data(withJSONObject: obj))
        }

        let gap = ProjectMigrator(currentVersion: 3, steps: [:])
        obj["schemaVersion"] = 1
        #expect(throws: ProjectMigrationError.noMigration(from: 1)) { try gap.migrate(JSONSerialization.data(withJSONObject: obj)) }
    }

    @Test func storeLoadAppliesMigration() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let migrator = ProjectMigrator(currentVersion: 2, steps: [1: { json in json["name"] = "migrated" }])
        let store = FileProjectStore(libraryURL: dir, migrator: migrator)
        let ref = try await store.createProject(named: "old")
        var obj = try JSONSerialization.jsonObject(with: JSONEncoder().encode(sampleProject(name: "old"))) as! [String: Any]
        obj["schemaVersion"] = 1
        try JSONSerialization.data(withJSONObject: obj).write(to: ref.url.appendingPathComponent("project.json"))
        #expect(try await store.load(ref).name == "migrated")
    }

    @Test func newerSchemaIsNotReplacedByBackup() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let ref = try await store.createProject(named: "n")
        try await store.save(sampleProject(), to: ref)
        try await store.save(sampleProject(name: "again"), to: ref)
        var obj = try JSONSerialization.jsonObject(with: JSONEncoder().encode(sampleProject())) as! [String: Any]
        obj["schemaVersion"] = 99
        try JSONSerialization.data(withJSONObject: obj).write(to: ref.url.appendingPathComponent("project.json"))
        await #expect(throws: NSSError.self) { try await store.load(ref) }
    }

    // MARK: Debounce (injected clock, no real sleeps)

    @Test func debounceWritesOnceAfterTheLastChange() async throws {
        let clock = ManualClock()
        let log = WriteLog()
        let d = AutosaveDebouncer(delay: 1, clock: clock) { p, r in log.add(p, r) }
        let ref = ProjectReference(url: URL(fileURLWithPath: "/tmp/a.nssproj"))
        d.schedule(sampleProject(name: "v1"), to: ref)
        await clock.waitForSleepers(1)
        clock.advance(by: 0.6)
        d.schedule(sampleProject(name: "v2"), to: ref)  // restarts the timer
        await clock.waitForSleepers(1)
        clock.advance(by: 0.6)  // 1.2 s since v1, only 0.6 s since v2
        await Task.yield()
        #expect(log.names.isEmpty)
        #expect(d.hasPending(for: ref))
        clock.advance(by: 0.5)
        await settle { !log.names.isEmpty }
        #expect(log.names == ["v2"])
        #expect(!d.hasPending(for: ref))
    }

    @Test func debounceKeepsBundlesIndependent() async throws {
        let clock = ManualClock()
        let log = WriteLog()
        let d = AutosaveDebouncer(delay: 1, clock: clock) { p, r in log.add(p, r) }
        d.schedule(sampleProject(name: "a"), to: ProjectReference(url: URL(fileURLWithPath: "/tmp/a.nssproj")))
        d.schedule(sampleProject(name: "b"), to: ProjectReference(url: URL(fileURLWithPath: "/tmp/b.nssproj")))
        await clock.waitForSleepers(2)
        clock.advance(by: 1)
        await settle { log.names.count == 2 }
        #expect(Set(log.names) == ["a", "b"])
    }

    @Test func flushWritesPendingImmediatelyAndTimerDoesNotWriteAgain() async throws {
        let clock = ManualClock()
        let log = WriteLog()
        let d = AutosaveDebouncer(delay: 1, clock: clock) { p, r in log.add(p, r) }
        let ref = ProjectReference(url: URL(fileURLWithPath: "/tmp/a.nssproj"))
        d.schedule(sampleProject(name: "v1"), to: ref)
        await clock.waitForSleepers(1)
        #expect(d.flush() == 1)
        #expect(log.names == ["v1"])
        clock.advance(by: 5)
        await Task.yield()
        #expect(log.names == ["v1"])
        #expect(d.flush() == 0)
    }

    @Test func cancelDropsPendingAndWriteErrorsAreReported() async throws {
        let clock = ManualClock()
        let log = WriteLog()
        let d = AutosaveDebouncer(delay: 1, clock: clock) { p, r in log.add(p, r) }
        let ref = ProjectReference(url: URL(fileURLWithPath: "/tmp/a.nssproj"))
        d.schedule(sampleProject(), to: ref)
        d.cancel(ref)
        #expect(d.flush() == 0)
        #expect(log.names.isEmpty)

        struct Boom: Error {}
        let errors = OSAllocatedUnfairLock(initialState: 0)
        let failing = AutosaveDebouncer(delay: 1, clock: clock, onError: { _, _ in errors.withLock { $0 += 1 } }) { _, _ in throw Boom() }
        failing.schedule(sampleProject(), to: ref)
        failing.flush()
        #expect(errors.withLock { $0 } == 1)
    }

    @Test func storeAutosaveWritesToDiskAfterDelayAndExplicitSaveSupersedes() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let clock = ManualClock()
        let store = FileProjectStore(libraryURL: dir, autosaveClock: clock)
        let ref = try await store.createProject(named: "auto")
        store.scheduleAutosave(sampleProject(name: "auto-1"), to: ref)
        store.scheduleAutosave(sampleProject(name: "auto-2"), to: ref)
        await clock.waitForSleepers(1)
        clock.advance(by: 1)
        await settle { !store.hasPendingAutosave(for: ref) }
        #expect(try await store.load(ref).name == "auto-2")

        store.scheduleAutosave(sampleProject(name: "stale"), to: ref)
        try await store.save(sampleProject(name: "explicit"), to: ref)
        #expect(!store.hasPendingAutosave(for: ref))
        #expect(store.flushPendingAutosaves() == 0)
        #expect(try await store.load(ref).name == "explicit")
    }

    @Test func explicitSaveDuringInFlightAutosaveWriteLandsLast() async throws {
        let clock = ManualClock()
        let log = WriteLog()
        let entered = OSAllocatedUnfairLock(initialState: false)
        let gate = DispatchSemaphore(value: 0)
        let d = AutosaveDebouncer(delay: 1, clock: clock) { p, r in
            entered.withLock { $0 = true }
            gate.wait()  // hold the autosave write "in flight"
            log.add(p, r)
        }
        let ref = ProjectReference(url: URL(fileURLWithPath: "/tmp/a.nssproj"))
        d.schedule(sampleProject(name: "P1"), to: ref)
        await clock.waitForSleepers(1)
        clock.advance(by: 1)
        await settle { entered.withLock { $0 } }
        // Explicit save arrives while P1 is being written: it must wait, then win.
        let explicit = Task.detached { d.exclusive(ref) { log.add(sampleProject(name: "P2"), ref) } }
        for _ in 0..<200 { await Task.yield() }
        #expect(log.names.isEmpty)
        gate.signal()
        await explicit.value
        #expect(log.names == ["P1", "P2"])
        #expect(!d.hasPending(for: ref))
    }

    @Test func explicitSaveBeatsAutosaveThatHasNotTakenItsEntryYet() async throws {
        let clock = ManualClock()
        let log = WriteLog()
        let d = AutosaveDebouncer(delay: 1, clock: clock) { p, r in log.add(p, r) }
        let ref = ProjectReference(url: URL(fileURLWithPath: "/tmp/a.nssproj"))
        d.schedule(sampleProject(name: "P1"), to: ref)
        await clock.waitForSleepers(1)
        d.exclusive(ref) { log.add(sampleProject(name: "P2"), ref) }
        clock.advance(by: 1)
        for _ in 0..<200 { await Task.yield() }
        #expect(log.names == ["P2"])
    }

    // MARK: Recents

    @Test func recentsListIsMruCappedAndPruned() {
        var r = RecentProjects(limit: 3)
        let d = Date(timeIntervalSince1970: 0)
        for n in ["a", "b", "c", "d"] { r.touch(path: "/\(n)", name: n, at: d) }
        #expect(r.items.map(\.name) == ["d", "c", "b"])
        r.touch(path: "/b", name: "b", at: d)
        #expect(r.items.map(\.name) == ["b", "d", "c"])
        #expect(r.prune { $0 != "/d" })
        #expect(r.items.map(\.name) == ["b", "c"])
        r.remove(path: "/b")
        #expect(r.items.map(\.name) == ["c"])
    }

    @Test func storeTracksRecentsAndPrunesDeletedBundles() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let a = try await store.createProject(named: "A")
        let b = try await store.createProject(named: "B")
        try await store.save(sampleProject(name: "A"), to: a)
        _ = try await store.load(a)
        #expect(await store.recentProjects().map(\.name) == ["A", "B"])
        try FileManager.default.removeItem(at: b.url)
        #expect(await store.recentProjects().map(\.name) == ["A"])
        // The recents file does not show up as a project.
        #expect(try await store.listProjects().count == 1)
    }
}
