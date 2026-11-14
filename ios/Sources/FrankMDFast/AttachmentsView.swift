import SwiftUI

enum AttachmentFilter: String, CaseIterable, Identifiable {
    case all = "Todos"
    case images = "Fotos"
    case documents = "Docs"
    case media = "Mídia"
    case other = "Outros"
    var id: String { rawValue }
}

struct AttachmentItem: Identifiable, Hashable, Sendable {
    let url: URL
    let relativePath: String
    let kind: AttachmentFilter
    var id: String { url.path }
    var name: String { url.lastPathComponent }
}

struct AttachmentsView: View {
    @EnvironmentObject private var vault: RustVaultStore
    @Environment(\.dismiss) private var dismiss
    let showsCloseButton: Bool

    @State private var items: [AttachmentItem] = []
    @State private var filter: AttachmentFilter = .all
    @State private var isLoading = true

    init(showsCloseButton: Bool = false) {
        self.showsCloseButton = showsCloseButton
    }

    private var filtered: [AttachmentItem] {
        filter == .all ? items : items.filter { $0.kind == filter }
    }

    var body: some View {
        Group {
            if isLoading {
                ProgressView("Procurando anexos…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if filtered.isEmpty {
                ContentUnavailableView(
                    filter == .all ? "Nenhum anexo" : "Nenhum anexo nesta categoria",
                    systemImage: "paperclip",
                )
            } else {
                List {
                    Section {
                        Picker("Filtrar", selection: $filter) {
                            ForEach(AttachmentFilter.allCases) { Text($0.rawValue).tag($0) }
                        }
                        .pickerStyle(.segmented)
                    }
                    Section {
                        ForEach(filtered) { item in
                            AttachmentRow(item: item)
                        }
                    }
                }
            }
        }
        .navigationTitle("Anexos")
        .toolbar {
            if showsCloseButton {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fechar") { dismiss() }
                }
            }
        }
        .task {
            let root = vault.root
            let scanned = await Task.detached(priority: .userInitiated) {
                Self.scan(root: root)
            }.value
            items = scanned
            isLoading = false
        }
    }

    nonisolated private static func scan(root: URL) -> [AttachmentItem] {
        let keys: Set<URLResourceKey> = [.isRegularFileKey, .isDirectoryKey, .isSymbolicLinkKey]
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: Array(keys),
            options: [.skipsHiddenFiles, .skipsPackageDescendants],
        ) else { return [] }

        let rootPath = root.standardizedFileURL.path
        var result: [AttachmentItem] = []
        for case let url as URL in enumerator {
            let values = try? url.resourceValues(forKeys: keys)
            guard values?.isDirectory != true,
                  values?.isRegularFile == true,
                  values?.isSymbolicLink != true,
                  url.pathExtension.lowercased() != "md"
            else { continue }

            let path = url.standardizedFileURL.path
            let relative = path.hasPrefix(rootPath + "/")
                ? String(path.dropFirst(rootPath.count + 1))
                : url.lastPathComponent
            let kind = category(for: url)
            result.append(AttachmentItem(url: url, relativePath: relative, kind: kind))
        }
        return result.sorted { $0.relativePath.localizedStandardCompare($1.relativePath) == .orderedAscending }
    }

    nonisolated private static func category(for url: URL) -> AttachmentFilter {
        switch url.pathExtension.lowercased() {
        case "png", "jpg", "jpeg", "heic", "gif", "webp", "tiff":
            .images
        case "pdf", "doc", "docx", "txt", "rtf", "csv", "xls", "xlsx":
            .documents
        case "mp4", "mov", "webm", "mp3", "m4a", "wav", "flac", "ogg":
            .media
        default:
            .other
        }
    }
}

private struct AttachmentRow: View {
    let item: AttachmentItem

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .foregroundStyle(NotesTheme.accent)
                .frame(width: 26)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.name).lineLimit(1)
                Text(item.relativePath)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            ShareLink(item: item.url) {
                Image(systemName: "square.and.arrow.up")
                    .accessibilityLabel("Compartilhar \(item.name)")
            }
        }
        .padding(.vertical, 2)
    }

    private var icon: String {
        switch item.kind {
        case .images: "photo"
        case .documents: "doc.richtext"
        case .media: "waveform"
        default: "paperclip"
        }
    }
}
