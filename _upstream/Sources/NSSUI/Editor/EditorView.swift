import SwiftUI
import NSSCore
import NSSExport
import NSSMotion
import NSSProject
import NSSRender

/// Everything alive while one project is open in the editor.
@MainActor
final class EditorSession {
    let model: EditorModel
    let engine: PreviewEngine
    let thumbnails = ThumbnailStore()

    init(model: EditorModel) {
        self.model = model
        engine = PreviewEngine(model: model)
        model.changeHandler = { [engine] change in engine.modelChanged(change) }
        thumbnails.load(url: model.reference.url.appendingPathComponent(model.project.assets.screenFile), duration: model.duration)
    }

    func close() {
        engine.teardown()
        thumbnails.cancel()
        model.changeHandler = nil
    }
}

/// Editor window for one project. `EditorView()` without arguments shows an empty state (used until the shell wires it up).
public struct EditorView: View {
    private let reference: ProjectReference?
    private let dependencies: EditorDependencies?
    private let onBack: (() -> Void)?
    @State private var session: EditorSession?
    @State private var loadError: String?

    public init() {
        reference = nil
        dependencies = nil
        onBack = nil
    }

    /// `onBack` shows a "Recordings" button in the toolbar that returns to the home screen.
    public init(reference: ProjectReference, dependencies: EditorDependencies, onBack: (() -> Void)? = nil) {
        self.reference = reference
        self.dependencies = dependencies
        self.onBack = onBack
    }

    public var body: some View {
        Group {
            if let session {
                EditorContent(session: session, onBack: onBack).id(ObjectIdentifier(session))
            } else if let loadError {
                VStack(spacing: 12) {
                    ContentUnavailableView("Couldn't open project", systemImage: "exclamationmark.triangle", description: Text(loadError))
                    if let onBack { Button("Back to recordings", action: onBack) }
                }
            } else if reference == nil {
                ContentUnavailableView("No project open", systemImage: "film", description: Text("Record something or open a project to edit it."))
            } else {
                ProgressView("Opening…")
            }
        }
        .frame(minWidth: 1040, minHeight: 700)
        .task(id: reference) { await load() }
        .onDisappear { session?.close() }
    }

    private func load() async {
        guard let reference, let dependencies else { return }
        do {
            let project = try await dependencies.store.load(reference)
            let track = try await dependencies.store.loadCursorTrack(for: reference, assets: project.assets)
            session?.close()
            if let library = dependencies.library {
                // Makes thumbnail.jpg for a fresh recording (cheap no-op when it exists), off the main thread.
                let summary = ProjectSummary(id: project.id, name: project.name, createdAt: project.createdAt,
                                             duration: project.assets.duration, bundle: reference)
                Task.detached(priority: .utility) { _ = await library.thumbnailURL(for: summary) }
            }
            session = EditorSession(model: EditorModel(project: project, reference: reference, track: track, dependencies: dependencies))
        } catch {
            loadError = error.localizedDescription
        }
    }
}

struct EditorContent: View {
    let session: EditorSession
    var onBack: (() -> Void)?
    @State private var exportModel: ExportModel?
    /// True while the project name field is being edited: transport and undo shortcuts then belong to the text field.
    @FocusState private var nameFocused: Bool

    private var model: EditorModel { session.model }
    private var engine: PreviewEngine { session.engine }

    var body: some View {
        VStack(spacing: 0) {
            EditorTopBar(model: model, onBack: onBack, export: { exportModel = ExportModel(editor: model) }, nameFocused: $nameFocused)
            Divider().overlay(EditorTheme.stroke)
            HStack(spacing: 0) {
                ZStack(alignment: .top) {
                    PreviewView(model: model, engine: engine)
                    if model.pickingFocusFor != nil {
                        Text("Click the preview to set the zoom focus")
                            .editorLabel(.row)
                            .padding(.horizontal, 10).padding(.vertical, 5)
                            .background(.regularMaterial, in: Capsule()).padding(8)
                    }
                }
                .frame(minWidth: 360, minHeight: 260)
                Divider().overlay(EditorTheme.stroke)
                InspectorView(model: model)
            }
            Divider().overlay(EditorTheme.stroke)
            VStack(spacing: EditorTheme.spacing) {
                transport
                EditorTimelineView(model: model, thumbnails: session.thumbnails, seek: { engine.seek(to: $0) })
            }
            .padding(.horizontal, EditorTheme.padding)
            .padding(.vertical, EditorTheme.spacing)
            .background(EditorTheme.panel)
        }
        .background(EditorTheme.canvas)
        .preferredColorScheme(.dark)
        .alert("Couldn't save", isPresented: Binding(get: { model.saveError != nil }, set: { if !$0 { model.saveError = nil } })) {
            Button("Stay") { model.saveError = nil }
            if let onBack { Button("Leave Anyway", role: .destructive) { model.saveError = nil; onBack() } }
        } message: { Text(model.saveError ?? "") }
        .sheet(item: $exportModel) { m in
            ExportSheet(export: m, close: { exportModel = nil })
        }
    }

    private var transport: some View {
        HStack(spacing: EditorTheme.spacingTight) {
            Button { engine.step(frames: -1) } label: { Image(systemName: "backward.frame.fill") }
                .keyboardShortcut(nameFocused ? nil : KeyboardShortcut(.leftArrow, modifiers: [])).help("Previous frame")
            Button { model.isPlaying ? engine.pause() : engine.play() } label: {
                Image(systemName: model.isPlaying ? "pause.fill" : "play.fill")
            }
            .keyboardShortcut(nameFocused ? nil : KeyboardShortcut(.space, modifiers: [])).help("Play / pause")
            Button { engine.step(frames: 1) } label: { Image(systemName: "forward.frame.fill") }
                .keyboardShortcut(nameFocused ? nil : KeyboardShortcut(.rightArrow, modifiers: [])).help("Next frame")
            TimeLabel(model: model).padding(.leading, EditorTheme.spacing)
        }
        .buttonStyle(.editorIcon)
        .frame(maxWidth: .infinity)
    }
}

/// Top bar: back to recordings, editable project name, undo/redo, aspect ratio picker and Export.
struct EditorTopBar: View {
    let model: EditorModel
    var onBack: (() -> Void)?
    var export: () -> Void
    @State private var nameDraft = ""
    var nameFocused: FocusState<Bool>.Binding

    var body: some View {
        HStack(spacing: EditorTheme.spacingLoose) {
            if let onBack {
                Button {
                    commitName()
                    // Write pending edits now so the home screen (name, thumbnail) sees them.
                    // Unchanged projects are not rewritten; a failed write is shown and keeps the editor open.
                    Task {
                        do { try await model.saveNow(); onBack() } catch {
                            model.saveError = "Couldn't save your changes: \(error.localizedDescription)"
                        }
                    }
                } label: { Label("Recordings", systemImage: "chevron.left") }
                    .buttonStyle(.editorSecondary)
                    .help("Back to all recordings")
            }
            TextField("Project name", text: $nameDraft)
                .textFieldStyle(.plain)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(EditorTheme.textPrimary)
                .padding(.horizontal, 8).frame(width: 220, height: 28)
                .background(nameFocused.wrappedValue ? EditorTheme.surface : Color.clear, in: RoundedRectangle(cornerRadius: EditorTheme.radiusMedium))
                .focused(nameFocused)
                .onSubmit { commitName() }
                .onChange(of: nameFocused.wrappedValue) { _, focused in if !focused { commitName() } }
                .onAppear { nameDraft = model.project.name }
                .onChange(of: model.project.name) { _, new in nameDraft = new }
                .help("Rename this project")
            Spacer()
            HStack(spacing: 0) {
                Button { model.undo() } label: { Image(systemName: "arrow.uturn.backward") }
                    .disabled(!model.canUndo).keyboardShortcut(nameFocused.wrappedValue ? nil : KeyboardShortcut("z", modifiers: .command)).help("Undo")
                Button { model.redo() } label: { Image(systemName: "arrow.uturn.forward") }
                    .disabled(!model.canRedo).keyboardShortcut(nameFocused.wrappedValue ? nil : KeyboardShortcut("z", modifiers: [.command, .shift])).help("Redo")
            }
            .buttonStyle(.editorIcon)
            Picker("Aspect ratio", selection: model.styleBinding(\.aspectRatio, name: "Aspect Ratio")) {
                ForEach(AspectRatio.allCases, id: \.self) { Text(InspectorBindings.label(for: $0)).tag($0) }
            }
            .pickerStyle(.segmented).labelsHidden().frame(width: 150)
            .help("Output aspect ratio")
            Button { commitName(); export() } label: { Label("Export", systemImage: "square.and.arrow.up") }
                .buttonStyle(.editorPrimary)
                .keyboardShortcut("e", modifiers: .command)
        }
        .padding(.horizontal, EditorTheme.padding)
        .frame(height: EditorTheme.topBarHeight)
        .background(EditorTheme.panel)
    }

    private func commitName() {
        nameFocused.wrappedValue = false
        model.setProjectName(nameDraft)
        nameDraft = model.project.name
    }
}

extension ExportModel: Identifiable {
    public nonisolated var id: ObjectIdentifier { ObjectIdentifier(self) }
}

/// "current / total" time. The only reader of `playhead` in the transport, so playback ticks don't re-evaluate `EditorContent`.
private struct TimeLabel: View {
    let model: EditorModel
    var body: some View {
        Text("\(TimelineGeometry.label(for: model.playhead)) / \(TimelineGeometry.label(for: model.duration))")
            .editorLabel(.value)
    }
}
