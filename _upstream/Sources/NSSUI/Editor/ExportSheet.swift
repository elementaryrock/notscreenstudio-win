import AppKit
import SwiftUI
import NSSCore

struct ExportSheet: View {
    @Bindable var export: ExportModel
    var close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
            HStack(spacing: EditorTheme.spacing) {
                Image(systemName: "square.and.arrow.up").foregroundStyle(EditorTheme.accent)
                Text("Export").font(.system(size: 17, weight: .semibold)).foregroundStyle(EditorTheme.textPrimary)
            }
            VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
                InspectorSegmentedRow("Resolution", selection: $export.resolution,
                                      options: [(ExportResolution.p1080, "1080p"), (ExportResolution.p2160, "4K")])
                InspectorSegmentedRow("Codec", selection: $export.codec,
                                      options: [(VideoCodec.h264, "H.264"), (VideoCodec.hevc, "HEVC")])
                HStack {
                    Text("Format").editorLabel(.row)
                    Spacer()
                    Text(export.outputDescription).editorLabel(.value)
                }
                HStack(spacing: EditorTheme.spacing) {
                    Text("Save to").editorLabel(.row)
                    Text(export.outputURL.path).editorLabel(.caption).lineLimit(1).truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                    Button("Choose…") { choose() }.buttonStyle(.editorSecondary)
                }
            }
            .padding(EditorTheme.padding)
            .background(EditorTheme.surface.opacity(0.55), in: RoundedRectangle(cornerRadius: EditorTheme.radiusLarge))
            .overlay(RoundedRectangle(cornerRadius: EditorTheme.radiusLarge).strokeBorder(EditorTheme.stroke))
            .disabled(export.isRunning)

            status

            HStack(spacing: EditorTheme.spacing) {
                Spacer()
                if export.isRunning {
                    Button("Cancel Export") { export.cancel() }.buttonStyle(.editorSecondary)
                } else if export.finishedURL != nil {
                    Button("Reveal in Finder") { export.revealInFinder() }.buttonStyle(.editorSecondary)
                    Button("Done") { close() }.buttonStyle(.editorPrimary).keyboardShortcut(.defaultAction)
                } else {
                    Button("Close") { close() }.buttonStyle(.editorSecondary).keyboardShortcut(.cancelAction)
                    Button("Export") { confirmAndStart() }.buttonStyle(.editorPrimary).keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(20)
        .frame(width: 460)
        .background(EditorTheme.panel)
        .preferredColorScheme(.dark)
    }

    @ViewBuilder private var status: some View {
        switch export.progress {
        case .running(let f, let written, let total):
            VStack(alignment: .leading) {
                ProgressView(value: f)
                Text(total > 0 ? "\(written) / \(total) frames" : "Starting…").editorLabel(.caption)
            }
        case .finished: Label("Export finished", systemImage: "checkmark.circle.fill").foregroundStyle(Color.green)
        case .failed(let m): Label(m, systemImage: "exclamationmark.triangle.fill").foregroundStyle(EditorTheme.destructive)
        case .cancelled: Label("Export cancelled", systemImage: "xmark.circle").foregroundStyle(EditorTheme.textSecondary)
        case .idle: EmptyView()
        }
    }

    private func confirmAndStart() {
        if FileManager.default.fileExists(atPath: export.outputURL.path) {
            let alert = NSAlert()
            alert.messageText = "Replace \"\(export.outputURL.lastPathComponent)\"?"
            alert.informativeText = "A file with this name already exists in that folder. Replacing it overwrites its contents."
            alert.addButton(withTitle: "Replace")
            alert.addButton(withTitle: "Cancel")
            guard alert.runModal() == .alertFirstButtonReturn else { return }
        }
        export.start()
    }

    private func choose() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.mpeg4Movie]
        panel.nameFieldStringValue = export.outputURL.lastPathComponent
        panel.directoryURL = export.outputURL.deletingLastPathComponent()
        if panel.runModal() == .OK, let url = panel.url {
            export.outputURL = url
            export.reset()
        }
    }
}
