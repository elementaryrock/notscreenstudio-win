import AppKit
import SwiftUI
import NSSCore
import NSSRender

/// The picker's tabs. Pure: which tab a background belongs to.
public enum BackgroundTab: String, CaseIterable, Identifiable, Sendable {
    case wallpapers = "Wallpapers", gradients = "Gradients", colours = "Colours", image = "Image"
    public var id: String { rawValue }

    public init(for background: Background) {
        switch background {
        case .systemWallpaper: self = .wallpapers
        case .gradient, .mesh, .wallpaper: self = .gradients
        case .solid: self = .colours
        case .image: self = .image
        }
    }

    /// Wallpapers and images have a blur slider.
    public var hasBlur: Bool { self == .wallpapers || self == .image }
}

/// Background section of the inspector: a tabbed grid of thumbnails. Every pick goes through
/// `EditorModel.setStyle`, so it is undoable, autosaved and shows in the preview at once.
struct BackgroundPicker: View {
    let model: EditorModel
    @State private var tab: BackgroundTab?
    @State private var wallpapers: [SystemWallpaper]?
    @State private var recents: [URL] = []
    @State private var importError: String?
    @State private var importing = false

    private let recentStore = RecentBackgroundImages()
    private var background: Background { model.project.style.background }
    private var currentTab: BackgroundTab { tab ?? BackgroundTab(for: background) }
    private let columns = [GridItem(.adaptive(minimum: 58, maximum: 80), spacing: EditorTheme.spacing)]

    var body: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
            InspectorSegmentedRow(
                selection: Binding(get: { currentTab }, set: { tab = $0 }),
                options: BackgroundTab.allCases.map { ($0, $0.rawValue) })

            switch currentTab {
            case .wallpapers: wallpaperGrid
            case .gradients: gradientGrid
            case .colours: colourGrid
            case .image: imageGrid
            }

            if currentTab.hasBlur {
                EditSlider(model: model, title: "Blur", value: model.styleBinding(\.backgroundBlur, name: "Background Blur"),
                           range: 0...1, format: InspectorFormat.percent)
                    .disabled(!background.isImageBased)
                    .opacity(background.isImageBased ? 1 : 0.4)
            }
        }
        .task {
            recents = await Task.detached { RecentBackgroundImages().load() }.value
        }
    }

    // MARK: Tabs

    @ViewBuilder private var wallpaperGrid: some View {
        if let wallpapers {
            if wallpapers.isEmpty {
                InspectorHint("No macOS wallpapers were found on this Mac.")
            } else {
                LazyVGrid(columns: columns, spacing: EditorTheme.spacing) {
                    ForEach(wallpapers) { w in
                        let bg = Background.systemWallpaper(path: w.url.path)
                        ThumbnailTile(source: .file(w.url), selected: background == bg, help: w.name) {
                            pick(bg, name: "Wallpaper")
                        }
                    }
                }
            }
        } else {
            ProgressView().controlSize(.small).frame(maxWidth: .infinity, minHeight: 44)
                .task { wallpapers = await Task.detached { SystemWallpaperCatalog.scan() }.value }
        }
    }

    private var gradientGrid: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
            LazyVGrid(columns: columns, spacing: EditorTheme.spacing) {
                ForEach(BackgroundPresets.gradients) { p in
                    ThumbnailTile(source: .background(p.background), selected: background == p.background, help: p.name) {
                        pick(p.background, name: "Gradient")
                    }
                }
            }
            if case .gradient(let stops, let angle) = background {
                EditSlider(model: model, title: "Angle", value: Binding(
                    get: { angle },
                    set: { model.setStyle(\.background, .gradient(stops: stops, angleDegrees: $0), name: "Gradient Angle") }
                ), range: 0...360, format: InspectorFormat.degrees)
            }
        }
    }

    private var colourGrid: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
            LazyVGrid(columns: columns, spacing: EditorTheme.spacing) {
                ForEach(BackgroundPresets.colors) { p in
                    ThumbnailTile(source: .background(p.background), selected: background == p.background, help: p.name) {
                        pick(p.background, name: "Background Colour")
                    }
                }
            }
            InspectorColorRow("Custom colour", color: Binding(
                get: {
                    if case .solid(let c) = background { return InspectorBindings.color(c) }
                    return InspectorBindings.color(model.project.style.backgroundFallbackColor)
                },
                set: { model.setStyle(\.background, .solid(InspectorBindings.rgba($0)), name: "Background Colour", coalescingKey: "bgcolor") }
            ))
        }
    }

    private var imageGrid: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
            LazyVGrid(columns: columns, spacing: EditorTheme.spacing) {
                Button(action: chooseImage) {
                    RoundedRectangle(cornerRadius: EditorTheme.radiusMedium).fill(EditorTheme.surface)
                        .overlay(RoundedRectangle(cornerRadius: EditorTheme.radiusMedium).strokeBorder(EditorTheme.stroke))
                        .overlay(Image(systemName: importing ? "hourglass" : "plus")
                            .font(.system(size: 14, weight: .medium)).foregroundStyle(EditorTheme.textSecondary))
                        .aspectRatio(16.0 / 10.0, contentMode: .fit)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Choose an image…")
                .disabled(importing)
                if case .image(let file) = background, !file.isEmpty {
                    ThumbnailTile(source: .file(projectFileURL(file)), selected: true, help: "Current image") {}
                }
                ForEach(recents, id: \.self) { url in
                    ThumbnailTile(source: .file(url), selected: false, help: url.lastPathComponent) { useImage(url) }
                        .contextMenu {
                            Button("Remove from Recents") {
                                recentStore.remove(url)
                                recents.removeAll { $0 == url }
                            }
                        }
                }
            }
            if recents.isEmpty, !background.isImageBased { InspectorHint("Images you choose are remembered here for every project.") }
            if let importError { Text(importError).editorLabel(.caption).foregroundStyle(EditorTheme.destructive) }
        }
    }

    // MARK: Actions

    private func projectFileURL(_ file: String) -> URL {
        file.hasPrefix("/") ? URL(fileURLWithPath: file) : model.reference.url.appendingPathComponent(file)
    }

    /// Applies `bg` and warms the renderer's image cache off the main thread, so the next preview frame does
    /// not wait for a large decode.
    private func pick(_ bg: Background, name: String) {
        guard bg != background else { return }
        model.setStyle(\.background, bg, name: name)
        guard bg.isImageBased, let renderer = model.dependencies.renderer as? MetalRenderer else { return }
        let style = model.project.style.resolvingAssets(relativeTo: model.reference.url)
        Task.detached(priority: .userInitiated) { renderer.preload(style: style) }
    }

    private func chooseImage() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = false
        panel.message = "Choose a background image"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        useImage(url)
    }

    /// Copies `url` into the project bundle (off the main thread), selects it and remembers it in Recents.
    private func useImage(_ url: URL) {
        let bundle = model.reference.url
        let store = recentStore
        importing = true
        Task {
            let result = await Task.detached(priority: .userInitiated) { () -> Result<String, Error> in
                let r = Result { try BackgroundImageImport.copy(url, into: bundle) }
                if case .success = r { store.add(url) }
                return r
            }.value
            importing = false
            switch result {
            case .success(let name):
                importError = nil
                recents = await Task.detached { store.load() }.value
                pick(.image(file: name), name: "Background Image")
            case .failure(let error):
                importError = "Couldn't use this image: \(error.localizedDescription)"
            }
        }
    }
}

/// One square-ish thumbnail with a selection ring.
private struct ThumbnailTile: View {
    enum Source: Equatable {
        case file(URL)
        case background(Background)
    }

    let source: Source
    let selected: Bool
    let help: String
    let action: () -> Void
    @State private var image: CGImage?

    var body: some View {
        Button(action: action) {
            ZStack {
                RoundedRectangle(cornerRadius: EditorTheme.radiusMedium).fill(EditorTheme.surface)
                if let image {
                    Image(decorative: image, scale: 2).resizable().scaledToFill()
                }
            }
            .aspectRatio(16.0 / 10.0, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: EditorTheme.radiusMedium))
            .overlay(RoundedRectangle(cornerRadius: EditorTheme.radiusMedium).strokeBorder(EditorTheme.stroke))
            .overlay(RoundedRectangle(cornerRadius: EditorTheme.radiusMedium + 3).strokeBorder(EditorTheme.accent, lineWidth: 2)
                .padding(-3).opacity(selected ? 1 : 0))
            .padding(3)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
        .accessibilityLabel(help)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .task(id: source) {
            switch source {
            case .file(let url): image = await BackgroundThumbnails.shared.thumbnail(forImageAt: url)
            case .background(let bg): image = await BackgroundThumbnails.shared.thumbnail(for: bg, width: 128, height: 80)
            }
        }
    }
}
