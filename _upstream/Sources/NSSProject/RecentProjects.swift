import Foundation

public struct RecentProject: Codable, Sendable, Equatable {
    public var path: String
    public var name: String
    public var lastOpened: Date
    public init(path: String, name: String, lastOpened: Date) {
        self.path = path
        self.name = name
        self.lastOpened = lastOpened
    }
}

/// Most-recently-used list of bundles. Pure value type; the store persists it.
public struct RecentProjects: Codable, Sendable, Equatable {
    public static let defaultLimit = 20
    public var items: [RecentProject]
    public var limit: Int

    public init(items: [RecentProject] = [], limit: Int = RecentProjects.defaultLimit) {
        self.items = items
        self.limit = limit
    }

    /// Moves (or adds) a project to the front and trims to `limit`.
    public mutating func touch(path: String, name: String, at date: Date) {
        items.removeAll { $0.path == path }
        items.insert(RecentProject(path: path, name: name, lastOpened: date), at: 0)
        if items.count > limit { items.removeLast(items.count - limit) }
    }

    public mutating func remove(path: String) { items.removeAll { $0.path == path } }

    /// Drops entries whose bundle no longer exists. Returns true if anything changed.
    @discardableResult
    public mutating func prune(exists: (String) -> Bool) -> Bool {
        let before = items.count
        items.removeAll { !exists($0.path) }
        return items.count != before
    }
}
