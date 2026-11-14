import SwiftUI

struct NotesBrowserView: View {
    @EnvironmentObject private var vault: RustVaultStore
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var hits: [RustSearchHit] = []

    private var rootNotes: [NoteEntry] {
        vault.notes.filter { $0.folder.isEmpty }
    }

    private var rootFolders: [String] {
        var folders = Set<String>()
        for note in vault.notes where !note.folder.isEmpty {
            folders.insert(note.folder.split(separator: "/")[0].description)
        }
        return folders.sorted()
    }

    var body: some View {
        NavigationStack {
            List {
                if query.isEmpty {
                    if !rootNotes.isEmpty {
                        Section("Raiz") {
                            ForEach(rootNotes) { note in
                                NoteRow(note: note) { dismiss() }
                            }
                        }
                    }
                    Section("Pastas") {
                        ForEach(rootFolders, id: \.self) { folder in
                            NavigationLink(value: folder) {
                                Label(folder, systemImage: "folder")
                            }
                        }
                    }
                } else {
                    if hits.isEmpty {
                        ContentUnavailableView.search(text: query)
                    } else {
                        ForEach(hits) { hit in
                            Button {
                                vault.open(hit.path)
                                dismiss()
                            } label: {
                                VStack(alignment: .leading, spacing: 3) {
                                    Label(
                                        hit.title,
                                        systemImage: hit.locked ? "lock.fill" : "doc.text",
                                    )
                                    Text(hit.path)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                    if !hit.snippet.isEmpty {
                                        Text(hit.snippet)
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                            .lineLimit(2)
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .searchable(text: $query, prompt: "Buscar no título e conteúdo")
            .task(id: query) {
                guard !query.trimmingCharacters(in: .whitespaces).isEmpty else {
                    hits = []
                    return
                }
                try? await Task.sleep(for: .milliseconds(250))
                guard !Task.isCancelled else { return }
                hits = vault.search(query)
            }
            .navigationTitle("Notas")
            .navigationDestination(for: String.self) { folder in
                FolderView(folder: folder)
                    .environmentObject(vault)
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Fechar") { dismiss() }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        vault.create()
                        dismiss()
                    } label: {
                        Label("Nova nota", systemImage: "plus")
                    }
                }
            }
        }
    }
}

private struct FolderView: View {
    @EnvironmentObject private var vault: RustVaultStore
    @Environment(\.dismiss) private var dismiss
    let folder: String

    private var prefix: String { folder + "/" }
    private var notes: [NoteEntry] {
        vault.notes.filter {
            $0.path.hasPrefix(prefix) &&
                !$0.path.dropFirst(prefix.count).contains("/")
        }
    }
    private var subfolders: [String] {
        var out = Set<String>()
        for note in vault.notes where note.path.hasPrefix(prefix) {
            let rest = note.path.dropFirst(prefix.count)
            if let slash = rest.firstIndex(of: "/") {
                out.insert(folder + "/" + rest[..<slash])
            }
        }
        return out.sorted()
    }

    var body: some View {
        List {
            if !notes.isEmpty {
                Section("Notas") {
                    ForEach(notes) { note in
                        NoteRow(note: note) { dismiss() }
                    }
                }
            }
            if !subfolders.isEmpty {
                Section("Pastas") {
                    ForEach(subfolders, id: \.self) { subfolder in
                        NavigationLink(value: subfolder) {
                            Label(
                                subfolder.split(separator: "/").last.map(String.init) ?? subfolder,
                                systemImage: "folder",
                            )
                        }
                    }
                }
            }
        }
        .navigationTitle(folder.split(separator: "/").last.map(String.init) ?? folder)
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct NoteRow: View {
    @EnvironmentObject private var vault: RustVaultStore
    let note: NoteEntry
    let onOpen: () -> Void

    var body: some View {
        Button {
            vault.open(note.path)
            onOpen()
        } label: {
            HStack(spacing: 10) {
                Image(systemName: note.locked ? "lock.fill" : note.encrypted ? "lock" : "doc.text")
                    .foregroundStyle(note.locked ? .orange : .secondary)
                    .frame(width: 20)
                VStack(alignment: .leading, spacing: 2) {
                    Text(note.title).foregroundStyle(.primary).lineLimit(1)
                    if note.path == vault.currentPath {
                        Text("Nota aberta").font(.caption2).foregroundStyle(.tint)
                    }
                }
                Spacer()
                if note.path == vault.currentPath {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(.tint)
                }
            }
            .contentShape(Rectangle())
        }
    }
}
