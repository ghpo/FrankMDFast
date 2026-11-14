import Foundation

struct NoteEntry: Identifiable, Hashable {
    let path: String
    let encrypted: Bool
    let locked: Bool
    let created: Date
    let modified: Date
    let snippet: String
    let displayTitle: String
    var id: String { path }
    var title: String { (path as NSString).lastPathComponent.replacingOccurrences(of: ".md", with: "") }
    var folder: String { (path as NSString).deletingLastPathComponent }

    init(
        path: String,
        encrypted: Bool = false,
        locked: Bool = false,
        created: Date = .distantPast,
        modified: Date = .distantPast,
        snippet: String = "",
        displayTitle: String? = nil
    ) {
        self.path = path
        self.encrypted = encrypted
        self.locked = locked
        self.created = created
        self.modified = modified
        self.snippet = snippet
        self.displayTitle = displayTitle ?? (path as NSString).lastPathComponent.replacingOccurrences(of: ".md", with: "")
    }
}

@MainActor
final class VaultStore: ObservableObject {
    @Published private(set) var notes: [NoteEntry] = []
    @Published var currentPath: String?
    @Published var text: String = ""
    @Published var savedText: String = ""
    @Published var isDirty: Bool = false
    @Published var status: String = ""

    let root: URL
    private var autosaveTask: Task<Void, Never>?

    init() {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        root = docs.appendingPathComponent("vault", isDirectory: true)
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        reload()
    }

    func reload(select: String? = nil) {
        let keys: [URLResourceKey] = [.isRegularFileKey, .isDirectoryKey]
        var found: [NoteEntry] = []
        if let enumerator = FileManager.default.enumerator(at: root, includingPropertiesForKeys: keys) {
            for case let url as URL in enumerator {
                guard url.pathExtension.lowercased() == "md" else { continue }
                let rel = url.path.replacingOccurrences(of: root.path + "/", with: "")
                found.append(NoteEntry(path: rel))
            }
        }
        notes = found.sorted { $0.path.localizedStandardCompare($1.path) == .orderedAscending }
        if let select, notes.contains(where: { $0.path == select }) {
            open(select)
        } else if currentPath == nil {
            currentPath = nil
            text = ""
            savedText = ""
            isDirty = false
        }
    }

    func open(_ path: String) {
        save()
        currentPath = path
        text = (try? String(contentsOf: url(path), encoding: .utf8)) ?? ""
        savedText = text
        isDirty = false
    }

    func updateText(_ value: String) {
        text = value
        isDirty = value != savedText
        autosaveTask?.cancel()
        autosaveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled else { return }
            self?.save()
        }
    }

    func save() {
        guard let path = currentPath, isDirty else { return }
        do {
            try text.write(to: url(path), atomically: true, encoding: .utf8)
            savedText = text
            isDirty = false
            status = "Salvo"
        } catch {
            status = "Falha ao salvar: \(error.localizedDescription)"
        }
    }

    func create() {
        var name = "Nova nota"
        var path = "\(name).md"
        var n = 2
        while FileManager.default.fileExists(atPath: url(path).path) {
            name = "Nova nota \(n)"
            path = "\(name).md"
            n += 1
        }
        try? "# \(name)\n\n".write(to: url(path), atomically: true, encoding: .utf8)
        reload(select: path)
    }

    func delete(_ path: String) {
        try? FileManager.default.trashItem(at: url(path), resultingItemURL: nil)
        if currentPath == path {
            currentPath = nil
            text = ""
            savedText = ""
            isDirty = false
        }
        reload()
    }

    func rename(_ old: String, to newName: String) {
        let trimmed = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.contains("/") else {
            status = "Nome inválido"
            return
        }
        let folder = (old as NSString).deletingLastPathComponent
        let newPath = folder.isEmpty ? "\(trimmed).md" : "\(folder)/\(trimmed).md"
        let destination = url(newPath)
        guard !FileManager.default.fileExists(atPath: destination.path) else {
            status = "Já existe uma nota com esse nome"
            return
        }
        do {
            try FileManager.default.moveItem(at: url(old), to: destination)
            reload(select: newPath)
        } catch {
            status = "Falha ao renomear: \(error.localizedDescription)"
        }
    }

    func url(_ path: String) -> URL {
        root.appendingPathComponent(path)
    }
}
