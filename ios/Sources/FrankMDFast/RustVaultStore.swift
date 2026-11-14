import CFrankMD
import Foundation

struct RustPreview {
    let html: String
    let base: String
}

struct RustSearchHit: Identifiable {
    let path: String
    let title: String
    let snippet: String
    let locked: Bool
    let created: Date
    let modified: Date
    var id: String { path }
}

@MainActor
final class RustVaultStore: ObservableObject {
    @Published private(set) var notes: [NoteEntry] = []
    @Published private(set) var folders: [String] = []
    @Published var currentPath: String?
    @Published var text: String = ""
    @Published var savedText: String = ""
    @Published var isDirty: Bool = false
    @Published var status: String = ""
    @Published private(set) var pinnedPaths: Set<String> = []
    private let pinDefaultsKey = "frankmd.pinned.paths"

    let root: URL
    private let state: URL
    nonisolated(unsafe) private var handle: OpaquePointer?
    private var autosaveTask: Task<Void, Never>?

    init() {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        root = docs.appendingPathComponent("vault", isDirectory: true)
        state = docs.appendingPathComponent("frankmd-state", isDirectory: true)
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try? FileManager.default.createDirectory(at: state, withIntermediateDirectories: true)
        pinnedPaths = Set(UserDefaults.standard.stringArray(forKey: pinDefaultsKey) ?? [])
        handle = root.path.withCString { rootPtr in
            state.path.withCString { statePtr in
                frankmd_ios_open(rootPtr, statePtr)
            }
        }
        reload()
    }

    deinit {
        if let handle { frankmd_ios_close(handle) }
    }

    var currentTitle: String {
        splitNote(text).title
    }

    var bodyText: String {
        splitNote(text).body
    }

    func updateTitle(_ value: String) {
        let parts = splitNote(text)
        updateText("# " + value + "\n\n" + parts.body)
    }

    func updateBody(_ value: String) {
        let parts = splitNote(text)
        updateText("# " + parts.title + "\n\n" + value)
    }

    func isPinned(_ path: String) -> Bool {
        pinnedPaths.contains(path)
    }

    func togglePin(_ path: String) {
        if pinnedPaths.contains(path) {
            pinnedPaths.remove(path)
        } else {
            pinnedPaths.insert(path)
        }
        UserDefaults.standard.set(Array(pinnedPaths), forKey: pinDefaultsKey)
    }

    func move(_ path: String, to folder: String) {
        save()
        let source = root.appendingPathComponent(path)
        let destination = root.appendingPathComponent(folder).appendingPathComponent((path as NSString).lastPathComponent)
        guard source != destination else { return }
        do {
            try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
            try FileManager.default.moveItem(at: source, to: destination)
            let newPath = folder.isEmpty ? destination.lastPathComponent : folder + "/" + destination.lastPathComponent
            if isPinned(path) {
                pinnedPaths.remove(path)
                pinnedPaths.insert(newPath)
                UserDefaults.standard.set(Array(pinnedPaths), forKey: pinDefaultsKey)
            }
            reload(select: newPath)
        } catch {
            status = "Falha ao mover: \(error.localizedDescription)"
        }
    }

    func renameFolder(_ folder: String, to newName: String) {
        let trimmed = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.contains("/") else {
            status = "Nome inválido"
            return
        }
        let source = root.appendingPathComponent(folder)
        let destination = source.deletingLastPathComponent().appendingPathComponent(trimmed)
        guard !FileManager.default.fileExists(atPath: destination.path) else {
            status = "Pasta já existe"
            return
        }
        do {
            try FileManager.default.moveItem(at: source, to: destination)
            reload()
        } catch {
            status = "Falha ao renomear pasta: \(error.localizedDescription)"
        }
    }

    func deleteFolder(_ folder: String) {
        let target = root.appendingPathComponent(folder)
        do {
            try FileManager.default.removeItem(at: target)
            if let currentPath, currentPath.hasPrefix(folder + "/") {
                self.currentPath = nil
                text = ""
                savedText = ""
                isDirty = false
            }
            reload()
        } catch {
            status = "Falha ao apagar pasta: \(error.localizedDescription)"
        }
    }

    private func splitNote(_ raw: String) -> (title: String, body: String) {
        var lines = raw.components(separatedBy: "\n")
        guard let first = lines.first else {
            return (currentPath.map { ($0 as NSString).lastPathComponent.replacingOccurrences(of: ".md", with: "") } ?? "Nota", "")
        }
        if first.hasPrefix("# ") {
            let title = String(first.dropFirst(2))
            lines.removeFirst()
            while lines.first?.isEmpty == true { lines.removeFirst() }
            return (title, lines.joined(separator: "\n"))
        }
        let fallback = currentPath.map { ($0 as NSString).lastPathComponent.replacingOccurrences(of: ".md", with: "") } ?? "Nota"
        return (fallback, raw)
    }

    func reload(select: String? = nil) {
        notes = listNotes()
        folders = scanFolders()
        if let select, notes.contains(where: { $0.path == select }) {
            open(select)
        } else if currentPath == nil {
            text = ""
            savedText = ""
            isDirty = false
        }
    }

    func open(_ path: String) {
        save()
        currentPath = path
        text = read(path) ?? ""
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
        if write(path, text) {
            savedText = text
            isDirty = false
            status = "Salvo"
        } else {
            status = "Falha ao salvar"
        }
    }

    func createFolder(_ name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.contains("/") else {
            status = "Nome de pasta inválido"
            return
        }
        let target = root.appendingPathComponent(trimmed, isDirectory: true)
        guard !FileManager.default.fileExists(atPath: target.path) else {
            status = "Pasta já existe"
            return
        }
        do {
            try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
            reload()
        } catch {
            status = "Falha ao criar pasta: \(error.localizedDescription)"
        }
    }

    private func scanFolders() -> [String] {
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey],
        ) else { return [] }
        var out: Set<String> = []
        let rootPath = root.resolvingSymlinksInPath().path
        for case let url as URL in enumerator {
            guard (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true else { continue }
            let path = url.resolvingSymlinksInPath().path
            guard path.hasPrefix(rootPath + "/") else { continue }
            let rel = String(path.dropFirst(rootPath.count + 1))
            if !rel.split(separator: "/").contains(where: { $0.hasPrefix(".") }) {
                out.insert(rel)
            }
        }
        return out.sorted { $0.localizedStandardCompare($1) == .orderedAscending }
    }

    @discardableResult
    func create(in folder: String? = nil) -> String? {
        guard let handle else { return nil }
        let raw = "Nova nota".withCString { frankmd_ios_create(handle, $0) }
        guard var path = takeString(raw) else {
            status = "Falha ao criar nota"
            return nil
        }
        if let folder, !folder.isEmpty {
            let source = root.appendingPathComponent(path)
            let destination = root.appendingPathComponent(folder).appendingPathComponent(path)
            try? FileManager.default.createDirectory(
                at: destination.deletingLastPathComponent(),
                withIntermediateDirectories: true,
            )
            try? FileManager.default.moveItem(at: source, to: destination)
            path = folder + "/" + path
        }
        reload(select: path)
        return path
    }

    func delete(_ path: String) {
        guard let handle else { return }
        let deleted = path.withCString { frankmd_ios_delete(handle, $0) }
        if deleted {
            if currentPath == path {
                currentPath = nil
                text = ""
                savedText = ""
                isDirty = false
            }
            reload()
        } else {
            status = "Falha ao apagar"
        }
    }

    func rename(_ old: String, to newName: String) {
        let trimmed = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.contains("/") else {
            status = "Nome inválido"
            return
        }
        let folder = (old as NSString).deletingLastPathComponent
        let newPath = folder.isEmpty ? "\(trimmed).md" : "\(folder)/\(trimmed).md"
        guard let handle else { return }
        let result = old.withCString { oldPtr in
            newPath.withCString { newPtr in
                frankmd_ios_rename(handle, oldPtr, newPtr)
            }
        }
        guard let path = takeString(result) else {
            status = "Falha ao renomear"
            return
        }
        reload(select: path)
    }

    func search(_ query: String) -> [RustSearchHit] {
        guard let handle, !query.trimmingCharacters(in: .whitespaces).isEmpty else { return [] }
        let raw = query.withCString { frankmd_ios_search(handle, $0, 100) }
        guard let json = takeString(raw),
              let data = json.data(using: .utf8),
              let rows = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else { return [] }
        return rows.compactMap { row in
            guard let path = row["path"] as? String else { return nil }
            let note = notes.first { $0.path == path }
            return RustSearchHit(
                path: path,
                title: row["title"] as? String ?? path,
                snippet: row["snippet"] as? String ?? "",
                locked: row["locked"] as? Bool ?? false,
                created: note?.created ?? .distantPast,
                modified: note?.modified ?? .distantPast,
            )
        }
    }

    func preview(_ path: String) -> RustPreview? {
        guard let handle else { return nil }
        let raw = path.withCString { frankmd_ios_preview(handle, $0) }
        guard let json = takeString(raw),
              let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: String],
              let html = object["html"], let base = object["base"]
        else { return nil }
        return RustPreview(html: html, base: base)
    }

    func cryptoStatus() -> (configured: Bool, unlocked: Bool) {
        guard let handle else { return (false, false) }
        let raw = frankmd_ios_status(handle)
        guard let json = takeString(raw),
              let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Bool]
        else { return (false, false) }
        return (object["configured"] ?? false, object["unlocked"] ?? false)
    }

    func unlock(_ password: String) -> Bool {
        guard let handle else { return false }
        return password.withCString { frankmd_ios_unlock(handle, $0) }
    }

    func lock() {
        guard let handle else { return }
        frankmd_ios_lock(handle)
        reload(select: currentPath)
    }

    @discardableResult
    func createSeal(_ password: String) -> Bool {
        guard let handle else { return false }
        let ok = password.withCString { frankmd_ios_create_seal(handle, $0) }
        status = ok ? "Criptografia criada e destravada" : "Não foi possível criar a criptografia"
        return ok
    }

    func readForSharing(_ path: String) -> String {
        if currentPath == path { return text }
        return read(path) ?? ""
    }

    func url(_ path: String) -> URL {
        root.appendingPathComponent(path)
    }

    @discardableResult
    func importSeal(_ json: String) -> Bool {
        guard let handle else { return false }
        let imported = json.withCString { frankmd_ios_import_seal(handle, $0) }
        if imported { reopenHandle() }
        return imported
    }

    @discardableResult
    func setEncrypted(_ path: String, _ encrypted: Bool) -> Bool {
        guard let handle else { return false }
        let ok = path.withCString { frankmd_ios_set_encrypted(handle, $0, encrypted) }
        if ok {
            reload(select: currentPath == path ? path : currentPath)
            status = encrypted ? "Nota criptografada" : "Nota descriptografada"
        } else {
            status = encrypted
                ? "Falha ao criptografar: configure/destrave a criptografia"
                : "Falha ao descriptografar: destrave a criptografia"
        }
        return ok
    }

    private func reopenHandle() {
        if let handle { frankmd_ios_close(handle) }
        handle = root.path.withCString { rootPtr in
            state.path.withCString { statePtr in
                frankmd_ios_open(rootPtr, statePtr)
            }
        }
        reload(select: currentPath)
    }

    private func listNotes() -> [NoteEntry] {
        guard let handle else { return [] }
        let raw = frankmd_ios_list(handle)
        guard let json = takeString(raw),
              let data = json.data(using: .utf8),
              let rows = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else { return [] }
        return rows.compactMap { row in
            guard let path = row["path"] as? String else { return nil }
            let url = root.appendingPathComponent(path)
            let values = try? url.resourceValues(forKeys: [.contentModificationDateKey, .creationDateKey])
            let modified = values?.contentModificationDate ?? .distantPast
            let created = values?.creationDate ?? modified
            let locked = row["locked"] as? Bool ?? false
            var snippet = ""
            var displayTitle: String?
            if !locked, let data = try? Data(contentsOf: url), let content = String(data: data, encoding: .utf8) {
                var lines = content.components(separatedBy: "\n")
                if let first = lines.first, first.hasPrefix("# ") {
                    displayTitle = String(first.dropFirst(2))
                    lines.removeFirst()
                }
                snippet = lines.joined(separator: " ").trimmingCharacters(in: .whitespacesAndNewlines)
            }
            return NoteEntry(
                path: path,
                encrypted: row["encrypted"] as? Bool ?? false,
                locked: locked,
                created: created,
                modified: modified,
                snippet: String(snippet.prefix(180)),
                displayTitle: displayTitle
            )
        }
        .sorted { $0.path.localizedStandardCompare($1.path) == .orderedAscending }
    }

    private func read(_ path: String) -> String? {
        guard let handle else { return nil }
        return path.withCString { takeString(frankmd_ios_read(handle, $0)) }
    }

    @discardableResult
    private func write(_ path: String, _ content: String) -> Bool {
        guard let handle else { return false }
        let result = path.withCString { pathPtr in
            content.withCString { contentPtr in
                frankmd_ios_write(handle, pathPtr, contentPtr)
            }
        }
        return takeString(result).map { !$0.isEmpty } ?? false
    }

    private func takeString(_ raw: UnsafeMutablePointer<CChar>?) -> String? {
        guard let raw else { return nil }
        defer { frankmd_ios_free_string(raw) }
        return String(cString: raw)
    }
}
