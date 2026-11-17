import AppKit
import NSSCore
import SwiftUI

/// Home screen: a grid of past recordings. Shown by the editor window when no project is open.
public struct HomeView: View {
    @State private var model: HomeModel
    private let onOpen: (ProjectReference) -> Void
    private let onRecord: () -> Void
    private let isRecording: Bool

    @State private var renaming: ProjectSummary?
    @State private var renameText = ""
    @State private var deleting: ProjectSummary?

    public init(
        library: any ProjectLibrary, isRecording: Bool = false,
        onOpen: @escaping (ProjectReference) -> Void, onRecord: @escaping () -> Void
    ) {
        _model = State(initialValue: HomeModel(library: library))
        self.isRecording = isRecording
        self.onOpen = onOpen
        self.onRecord = onRecord
    }

    public var body: some View {
        Group {
            switch model.state {
            case .loading:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            case .failed(let message):
                ContentUnavailableView("Couldn't load recordings", systemImage: "exclamationmark.triangle", description: Text(message))
            case .loaded:
                if model.items.isEmpty { emptyState } else { grid }
            }
        }
        .frame(minWidth: 720, minHeight: 480)
        .background(EditorTheme.canvas)
        .preferredColorScheme(.dark)
        .task { await model.reload() }
        // Pick up recordings finished while this window stays open.
        .onChange(of: isRecording) { _, now in if !now { Task { await model.reload() } } }
        .toolbar {
            ToolbarItem {
                Button(action: onRecord) { Label("Record", systemImage: "record.circle") }
                    .disabled(isRecording).help("Start a new recording")
            }
        }
        .alert("Rename recording", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $renameText)
            Button("Rename") {
                if let s = renaming { Task { await model.rename(s, to: renameText) } }
                renaming = nil
            }
            Button("Cancel", role: .cancel) { renaming = nil }
        }
        .alert("Move to Trash?", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } })) {
            Button("Move to Trash", role: .destructive) {
                if let s = deleting { Task { await model.delete(s) } }
                deleting = nil
            }
            Button("Cancel", role: .cancel) { deleting = nil }
        } message: {
            Text("“\(deleting?.name ?? "")” will be moved to the Trash. You can restore it from there.")
        }
        .alert("Something went wrong", isPresented: Binding(get: { model.actionError != nil }, set: { if !$0 { model.actionError = nil } })) {
            Button("OK") { model.actionError = nil }
        } message: {
            Text(model.actionError ?? "")
        }
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label("No recordings yet", systemImage: "film.stack")
        } description: {
            Text("Record your screen and it will show up here.")
        } actions: {
            Button(action: onRecord) { Label("Record", systemImage: "record.circle") }
                .buttonStyle(.editorPrimary).disabled(isRecording)
        }
    }

    private var grid: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 220, maximum: 320), spacing: 20)], spacing: 24) {
                ForEach(model.items) { item in
                    HomeTile(summary: item, image: model.thumbnails[item.id], failed: model.failedThumbnails.contains(item.id))
                        .onTapGesture { onOpen(item.bundle) }
                        .task(id: item) { await model.loadThumbnail(for: item) }
                        .contextMenu {
                            Button("Open") { onOpen(item.bundle) }
                            Button("Rename…") { renameText = item.name; renaming = item }
                            Button("Reveal in Finder") { NSWorkspace.shared.activateFileViewerSelecting([item.bundle.url]) }
                            Divider()
                            Button("Move to Trash", role: .destructive) { deleting = item }
                        }
                }
            }
            .padding(24)
        }
    }
}

/// One recording in the grid.
struct HomeTile: View {
    let summary: ProjectSummary
    let image: NSImage?
    let failed: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack {
                Rectangle().fill(EditorTheme.surface)
                if let image {
                    Image(nsImage: image).resizable().aspectRatio(contentMode: .fill)
                } else if failed {
                    Image(systemName: "film").font(.largeTitle).foregroundStyle(.secondary)
                } else {
                    ProgressView().controlSize(.small)
                }
            }
            .aspectRatio(16.0 / 10.0, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: EditorTheme.radiusMedium))
            .overlay(RoundedRectangle(cornerRadius: EditorTheme.radiusMedium).strokeBorder(EditorTheme.stroke))
            .overlay(alignment: .bottomTrailing) {
                Text(HomeModel.durationLabel(summary.duration))
                    .font(.caption2.monospacedDigit()).foregroundStyle(.white)
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(.black.opacity(0.65), in: Capsule()).padding(6)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(summary.name).editorLabel(.header).lineLimit(1)
                Text(summary.createdAt.formatted(date: .abbreviated, time: .shortened))
                    .editorLabel(.caption)
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }
}
