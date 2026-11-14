import SwiftUI

enum NoteSort: String, CaseIterable, Identifiable {
    case editedNewest = "Editadas recentemente"
    case createdOldest = "Mais antigas"
    case title = "Título"
    var id: String { rawValue }
}

enum NoteLayout: String, CaseIterable, Identifiable {
    case list = "Lista"
    case gallery = "Galeria"
    var id: String { rawValue }
}

struct NotesListView: View {
    @EnvironmentObject private var vault: RustVaultStore
    let folder: String?
    @Binding var path: [FolderRoute]
    @State private var query = ""
    @State private var hits: [RustSearchHit] = []
    @State private var sort: NoteSort = .editedNewest
    @State private var layout: NoteLayout = .list
    @State private var moveTarget: NoteEntry?
    @State private var showMove = false
    @State private var moveFolder = ""

    private var title: String { folder ?? "Todas as notas" }

    private var visibleNotes: [NoteEntry] {
        let filtered: [NoteEntry]
        if !query.isEmpty {
            let paths = Set(hits.map(\.path))
            filtered = vault.notes.filter { note in
                guard paths.contains(note.path) else { return false }
                guard let folder else { return true }
                return note.folder == folder || note.path.hasPrefix(folder + "/")
            }
        } else {
            filtered = vault.notes.filter { note in
                guard let folder else { return true }
                return note.folder == folder
            }
        }
        return filtered.sorted { lhs, rhs in
            let lp = vault.isPinned(lhs.path)
            let rp = vault.isPinned(rhs.path)
            if lp != rp { return lp }
            switch sort {
            case .editedNewest: return lhs.modified > rhs.modified
            case .createdOldest: return lhs.created < rhs.created
            case .title: return lhs.displayTitle.localizedStandardCompare(rhs.displayTitle) == .orderedAscending
            }
        }
    }

    private var subfolders: [String] {
        guard let folder else { return [] }
        let prefix = folder + "/"
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
        Group {
            if layout == .list { listView } else { galleryView }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.large)
        .searchable(text: $query, prompt: "Pesquisar")
        .task(id: query) {
            guard !query.trimmingCharacters(in: .whitespaces).isEmpty else {
                hits = []
                return
            }
            try? await Task.sleep(for: .milliseconds(250))
            if !Task.isCancelled { hits = vault.search(query) }
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Menu {
                    Section("Ordenar") {
                        Picker("Ordenar", selection: $sort) {
                            ForEach(NoteSort.allCases) { Text($0.rawValue).tag($0) }
                        }
                    }
                    Section("Visualização") {
                        Picker("Visualização", selection: $layout) {
                            ForEach(NoteLayout.allCases) { Text($0.rawValue).tag($0) }
                        }
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                Button {
                    if let created = vault.create(in: folder) {
                        path.append(.note(created))
                    }
                } label: {
                    Image(systemName: "square.and.pencil")
                }
            }
        }
        .sheet(isPresented: $showMove) {
            NavigationStack {
                List {
                    Button("Todas as notas") { performMove(to: "") }
                    ForEach(vault.folders, id: \.self) { folder in
                        Button(folder) { performMove(to: folder) }
                    }
                }
                .navigationTitle("Mover nota")
                .toolbar { Button("Cancelar") { showMove = false } }
            }
            .presentationDetents([.medium, .large])
        }
    }

    private var listView: some View {
        List {
            if query.isEmpty && !subfolders.isEmpty {
                Section("Pastas") {
                    ForEach(subfolders, id: \.self) { subfolder in
                        NavigationLink(value: FolderRoute.folder(subfolder)) {
                            Label(
                                subfolder.split(separator: "/").last.map(String.init) ?? subfolder,
                                systemImage: "folder",
                            )
                        }
                    }
                }
            }
            Section {
                if visibleNotes.isEmpty {
                    ContentUnavailableView(
                        query.isEmpty ? "Nenhuma nota" : "Nenhum resultado",
                        systemImage: query.isEmpty ? "note.text" : "magnifyingglass",
                    )
                    .listRowBackground(Color.clear)
                }
                ForEach(visibleNotes) { note in
                    NavigationLink(value: FolderRoute.note(note.path)) {
                        NoteCard(note: note, pinned: vault.isPinned(note.path))
                    }
                    .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                        Button(role: .destructive) { vault.delete(note.path) } label: {
                            Label("Apagar", systemImage: "trash")
                        }
                    }
                    .swipeActions(edge: .leading) {
                        Button {
                            vault.togglePin(note.path)
                        } label: {
                            Label(vault.isPinned(note.path) ? "Desafixar" : "Fixar", systemImage: "pin")
                        }
                        .tint(NotesTheme.accent)
                    }
                    .contextMenu {
                        Button(vault.isPinned(note.path) ? "Desafixar" : "Fixar") {
                            vault.togglePin(note.path)
                        }
                        Button("Mover") {
                            moveTarget = note
                            moveFolder = note.folder
                            showMove = true
                        }
                        ShareLink(item: vault.readForSharing(note.path)) {
                            Label("Compartilhar", systemImage: "square.and.arrow.up")
                        }
                        Button("Apagar", role: .destructive) { vault.delete(note.path) }
                    }
                }
            }
        }
        .listStyle(.plain)
    }

    private var galleryView: some View {
        ScrollView {
            if visibleNotes.isEmpty {
                ContentUnavailableView("Nenhuma nota", systemImage: "note.text")
                    .padding(.top, 80)
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 14)], spacing: 14) {
                    ForEach(visibleNotes) { note in
                        NavigationLink(value: FolderRoute.note(note.path)) {
                            NoteGalleryCard(note: note, pinned: vault.isPinned(note.path))
                        }
                        .buttonStyle(.plain)
                        .contextMenu {
                            Button(vault.isPinned(note.path) ? "Desafixar" : "Fixar") {
                                vault.togglePin(note.path)
                            }
                            Button("Mover") {
                                moveTarget = note
                                moveFolder = note.folder
                                showMove = true
                            }
                            Button("Apagar", role: .destructive) { vault.delete(note.path) }
                        }
                    }
                }
                .padding(16)
            }
        }
        .background(NotesTheme.listBackground)
    }

    private func performMove(to destination: String) {
        guard let moveTarget else { return }
        showMove = false
        vault.move(moveTarget.path, to: destination)
    }
}

private struct NoteCard: View {
    let note: NoteEntry
    let pinned: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: pinned ? "pin.fill" : note.locked ? "lock.fill" : note.encrypted ? "lock" : "doc.text")
                .foregroundStyle(pinned ? NotesTheme.accent : note.locked ? .orange : NotesTheme.accent)
                .font(.title3)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 4) {
                Text(note.displayTitle)
                    .font(.headline)
                    .lineLimit(1)
                if !note.snippet.isEmpty {
                    Text(note.snippet)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                NoteDatesView(created: note.created, modified: note.modified)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 4)
    }
}

private struct NoteGalleryCard: View {
    let note: NoteEntry
    let pinned: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Image(systemName: pinned ? "pin.fill" : note.locked ? "lock.fill" : "doc.text")
                    .foregroundStyle(NotesTheme.accent)
                Spacer()
                NoteDatesView(created: note.created, modified: note.modified)
            }
            Text(note.displayTitle)
                .font(.headline)
                .lineLimit(2)
            Text(note.snippet.isEmpty ? "Sem prévia" : note.snippet)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(4)
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, minHeight: 150, alignment: .topLeading)
        .padding(14)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}
