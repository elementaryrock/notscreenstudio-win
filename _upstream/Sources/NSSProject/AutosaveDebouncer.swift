import Foundation
import NSSCore
import os

/// Time source for the debounce, injectable so tests never sleep for real.
public protocol DebounceClock: Sendable {
    /// Suspends for `seconds`; throws `CancellationError` when the task is cancelled.
    func sleep(for seconds: Double) async throws
}

public struct SystemDebounceClock: DebounceClock {
    public init() {}
    public func sleep(for seconds: Double) async throws {
        try await Task.sleep(for: .seconds(seconds))
    }
}

/// Debounced autosave. Each `schedule` replaces any pending save for the same bundle and restarts the timer;
/// the write happens `delay` seconds after the LAST change. `flush()` writes everything pending immediately
/// (call it on app quit).
///
/// It is a lock-protected class rather than an actor on purpose: `ProjectStore.scheduleAutosave` is a
/// synchronous call, and an actor hop through a `Task` could reorder two quick calls so the older project wins.
/// Here the ordering is decided synchronously under the lock, and `flush()` is synchronous so it can run
/// inside `applicationWillTerminate`.
public final class AutosaveDebouncer: Sendable {
    public typealias Writer = @Sendable (Project, ProjectReference) throws -> Void
    public typealias ErrorHandler = @Sendable (Error, ProjectReference) -> Void

    private struct Pending {
        var project: Project
        var generation: Int
        var task: Task<Void, Never>?
    }

    private let delay: Double
    private let clock: any DebounceClock
    private let writer: Writer
    private let onError: ErrorHandler?
    private let state = OSAllocatedUnfairLock<[URL: Pending]>(initialState: [:])
    private let counter = OSAllocatedUnfairLock<Int>(initialState: 0)
    /// Serializes every write (autosave fire, flush, explicit save). A pending entry is taken and written
    /// while holding it, so a superseded autosave can never land after a newer save.
    private let writeLock = NSLock()

    public init(
        delay: Double = 1,
        clock: any DebounceClock = SystemDebounceClock(),
        onError: ErrorHandler? = nil,
        writer: @escaping Writer
    ) {
        self.delay = delay
        self.clock = clock
        self.writer = writer
        self.onError = onError
    }

    public func schedule(_ project: Project, to ref: ProjectReference) {
        let generation = counter.withLock { $0 += 1; return $0 }
        let key = ref.url.standardizedFileURL
        // Store the entry first, so `cancel`/`flush` can see it before its timer task exists.
        let old = state.withLock { s -> Task<Void, Never>? in
            let previous = s[key]?.task
            s[key] = Pending(project: project, generation: generation, task: nil)
            return previous
        }
        old?.cancel()
        let task = Task { [delay, clock] in
            do { try await clock.sleep(for: delay) } catch { return }
            self.fire(key: key, ref: ref, generation: generation)
        }
        let attached = state.withLock { s -> Bool in
            guard s[key]?.generation == generation else { return false }
            s[key]?.task = task
            return true
        }
        if !attached { task.cancel() }
    }

    /// Drops the pending save for `ref`, for example because an explicit save just superseded it.
    public func cancel(_ ref: ProjectReference) {
        let old = state.withLock { $0.removeValue(forKey: ref.url.standardizedFileURL) }
        old?.task?.cancel()
    }

    /// Runs an explicit save exclusively: waits for any in-flight autosave write to finish, drops the pending
    /// autosave for `ref`, then runs `body`. Whatever `body` writes is therefore the last write for that bundle.
    public func exclusive<T>(_ ref: ProjectReference, _ body: () throws -> T) rethrows -> T {
        writeLock.lock()
        defer { writeLock.unlock() }
        cancel(ref)
        return try body()
    }

    public func hasPending(for ref: ProjectReference) -> Bool {
        state.withLock { $0[ref.url.standardizedFileURL] != nil }
    }

    /// Writes every pending project now, synchronously. Returns the number written.
    @discardableResult
    public func flush() -> Int {
        writeLock.lock()
        defer { writeLock.unlock() }
        let all = state.withLock { s -> [(URL, Pending)] in
            let items = Array(s)
            s.removeAll()
            return items.map { ($0.key, $0.value) }
        }
        for (url, p) in all {
            p.task?.cancel()
            write(p.project, ProjectReference(url: url))
        }
        return all.count
    }

    private func fire(key: URL, ref: ProjectReference, generation: Int) {
        writeLock.lock()
        defer { writeLock.unlock() }
        // Only the newest scheduled save may write; a flush or newer call may already have taken it.
        let taken = state.withLock { s -> Pending? in
            guard let p = s[key], p.generation == generation else { return nil }
            s[key] = nil
            return p
        }
        if let taken { write(taken.project, ref) }
    }

    private func write(_ project: Project, _ ref: ProjectReference) {
        do { try writer(project, ref) } catch { onError?(error, ref) }
    }
}
