import SwiftUI

enum FolderRoute: Hashable {
    case all
    case folder(String)
    case note(String)
    case attachments
}

struct FoldersView: View {
    @EnvironmentObject private var vault: RustVaultStore
    @State private var path: [FolderRoute] = []
    @State private var showSettings = false
    @State private var showNewFolder = false
    @State private var folderName = ""
    @State private var query = ""
    @State private var hits: [RustSearchHit] = []
    @State private var editingFolder: String?
    @State private var renameFolderName = ""
    @State private var deletingFolder: String?
    @State private var showRenameFolder = false
    @State private var showDeleteFolder = false

    private var topFolders: [String] {
        Array(Set(vault.folders.compactMap { folder -> String? in
            folder.split(separator: "/").first.map(String.init)
        })).sorted()
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if query.isEmpty {
                    Section("iCloud") {
                        NavigationLink(value: FolderRoute.all) {
                            FolderRow(
                                title: "Todas as notas",
                                systemImage: "folder",
                                count: vault.notes.count,
                            )
                        }
                        ForEach(topFolders, id: \.self) { folder in
                            NavigationLink(value: FolderRoute.folder(folder)) {
                                FolderRow(
                                    title: folder,
                                    systemImage: "folder",
                                    count: vault.notes.filter { $0.folder == folder || $0.folder.hasPrefix(folder + "/") }.count,
                                )
                            }
                            .contextMenu {
                                Button("Renomear pasta") {
                                    editingFolder = folder
                                    renameFolderName = folder.split(separator: "/").last.map(String.init) ?? folder
                                    showRenameFolder = true
                                }
                                Button("Apagar pasta", role: .destructive) {
                                    deletingFolder = folder
                                    showDeleteFolder = true
                                }
                            }
                        }
                    }
                    Section("Inteligentes") {
                        NavigationLink(value: FolderRoute.attachments) {
                            FolderRow(title: "Anexos", systemImage: "paperclip", count: nil)
                        }
                    }
                } else if hits.isEmpty {
                    ContentUnavailableView.search(text: query)
                } else {
                    ForEach(hits) { hit in
                        NavigationLink(value: FolderRoute.note(hit.path)) {
                            SearchHitRow(
                                hit: hit,
                                note: vault.notes.first { $0.path == hit.path },
                            )
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle("Pastas")
            .searchable(text: $query, prompt: "Pesquisar")
            .task(id: query) {
                guard !query.trimmingCharacters(in: .whitespaces).isEmpty else {
                    hits = []
                    return
                }
                try? await Task.sleep(for: .milliseconds(250))
                if !Task.isCancelled { hits = vault.search(query) }
            }
            .navigationDestination(for: FolderRoute.self) { route in
                switch route {
                case .all:
                    NotesListView(folder: nil, path: $path)
                case .folder(let folder):
                    NotesListView(folder: folder, path: $path)
                case .note(let note):
                    EditorView(path: note)
                case .attachments:
                    AttachmentsView()
                }
            }
            .overlay(alignment: .bottomTrailing) {
                Button {
                    if let created = vault.create(in: nil) {
                        path.append(.note(created))
                    }
                } label: {
                    Image(systemName: "square.and.pencil")
                        .font(.title3.weight(.semibold))
                        .frame(width: 52, height: 52)
                        .foregroundStyle(.white)
                        .background(Color.accentColor, in: Circle())
                        .shadow(color: .black.opacity(0.18), radius: 8, y: 4)
                }
                .padding(.trailing, 20)
                .padding(.bottom, 18)
                .accessibilityLabel("Nova nota")
            }
            .toolbar {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button {
                        folderName = ""
                        showNewFolder = true
                    } label: {
                        Image(systemName: "folder.badge.plus")
                    }
                    Button { showSettings = true } label: {
                        Image(systemName: "gearshape")
                    }
                }
            }
            .sheet(isPresented: $showSettings) {
                SettingsView().environmentObject(vault)
            }
            .alert("Nova pasta", isPresented: $showNewFolder) {
                TextField("Nome", text: $folderName)
                Button("Cancelar", role: .cancel) {}
                Button("Criar") { vault.createFolder(folderName) }
            }
            .alert("Renomear pasta", isPresented: $showRenameFolder) {
                TextField("Nome", text: $renameFolderName)
                Button("Cancelar", role: .cancel) {}
                Button("Renomear") {
                    if let editingFolder { vault.renameFolder(editingFolder, to: renameFolderName) }
                }
            }
            .confirmationDialog(
                "Apagar a pasta \(deletingFolder ?? "") e todas as notas dentro dela?",
                isPresented: $showDeleteFolder,
                titleVisibility: .visible,
            ) {
                Button("Apagar pasta", role: .destructive) {
                    if let deletingFolder { vault.deleteFolder(deletingFolder) }
                }
                Button("Cancelar", role: .cancel) {}
            }
        }
    }
}

private struct SearchHitRow: View {
    let hit: RustSearchHit
    let note: NoteEntry?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(hit.title, systemImage: hit.locked ? "lock.fill" : "doc.text")
                .font(.headline)
            if !hit.snippet.isEmpty {
                Text(hit.snippet)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            Text(hit.path)
                .font(.caption2)
                .foregroundStyle(.secondary)
            NoteDatesView(
                created: note?.created ?? hit.created,
                modified: note?.modified ?? hit.modified,
                compact: true,
            )
        }
    }
}

private struct FolderRow: View {
    let title: String
    let systemImage: String
    let count: Int?

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: systemImage)
                .foregroundStyle(Color.accentColor)
                .font(.title3)
                .frame(width: 26)
            Text(title)
                .font(.body.weight(.medium))
            Spacer()
            if let count {
                Text(String(count))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            Image(systemName: "chevron.right")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
    }
}
