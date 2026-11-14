import CryptoKit
import Foundation

struct SyncResult {
    let downloaded: Int
    let uploaded: Int
    let deleted: Int
    let conflicts: Int
}

private struct RemoteFile: Decodable {
    let path: String
    let revision: String
}

private struct Manifest: Decodable {
    let files: [RemoteFile]
}

private struct RevisionResponse: Decodable {
    let revision: String
}

enum SyncError: LocalizedError {
    case invalidURL
    case http(String, Int)
    case server(String)

    var errorDescription: String? {
        switch self {
        case .invalidURL: "URL do servidor inválida"
        case let .http(path, code): "HTTP \(code): \(path)"
        case let .server(message): message
        }
    }
}

final class SyncManager: @unchecked Sendable {
    static let shared = SyncManager()
    private let baseKey = "frankmd.syncBase"

    @MainActor
    func sync(
        vault: RustVaultStore,
        serverURL: String,
        token: String,
        onProgress: ((Int, Int) -> Void)? = nil,
    ) async throws -> SyncResult {
        guard let base = URL(string: serverURL.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            throw SyncError.invalidURL
        }
        let token = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !token.isEmpty else { throw SyncError.server("Token não configurado") }

        let manifestURL = base.appendingPathComponent("api/sync/manifest")
        var request = URLRequest(url: manifestURL)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let (data, response) = try await URLSession.shared.data(for: request)
        try check(response, path: "manifest")
        let remote = try JSONDecoder().decode(Manifest.self, from: data).files
        var remoteByPath = Dictionary(uniqueKeysWithValues: remote.map { ($0.path, $0.revision) })

        let local = localFiles(vault.root)
        let baseMap = loadBase()
        var nextBase: [String: String] = [:]
        var downloaded = 0
        var uploaded = 0
        var deleted = 0
        var conflicts = 0

        let paths = Set(remoteByPath.keys).union(local.keys).union(baseMap.keys).sorted()
        onProgress?(0, paths.count)
        for (index, path) in paths.enumerated() {
            onProgress?(index + 1, paths.count)
            let remoteRev = remoteByPath.removeValue(forKey: path)
            let localURL = local[path]
            let localRev = localURL.flatMap { sha($0) }
            let baseRev = baseMap[path]

            if let localURL, localRev != nil, remoteRev == nil {
                if let baseRev, localRev == baseRev {
                    try? FileManager.default.removeItem(at: localURL)
                    deleted += 1
                } else {
                    nextBase[path] = try await upload(base: base, token: token, path: path, file: localURL)
                    uploaded += 1
                }
            } else if localURL == nil, let remoteRev {
                if let baseRev, remoteRev == baseRev {
                    try await deleteRemote(base: base, token: token, path: path, revision: remoteRev)
                    deleted += 1
                } else {
                    try await download(base: base, token: token, path: path, destination: vault.url(path))
                    nextBase[path] = remoteRev
                    downloaded += 1
                }
            } else if let localURL, let localRev, let remoteRev {
                if localRev == remoteRev {
                    nextBase[path] = remoteRev
                } else if localRev == baseRev {
                    try await download(base: base, token: token, path: path, destination: localURL)
                    nextBase[path] = remoteRev
                    downloaded += 1
                } else if remoteRev == baseRev {
                    nextBase[path] = try await upload(
                        base: base, token: token, path: path, file: localURL, expected: remoteRev,
                    )
                    uploaded += 1
                } else {
                    try? FileManager.default.copyItem(at: localURL, to: conflictURL(localURL))
                    try await download(base: base, token: token, path: path, destination: localURL)
                    nextBase[path] = remoteRev
                    conflicts += 1
                }
            }
        }
        saveBase(nextBase)
        return SyncResult(
            downloaded: downloaded,
            uploaded: uploaded,
            deleted: deleted,
            conflicts: conflicts,
        )
    }

    private func localFiles(_ root: URL) -> [String: URL] {
        let keys: [URLResourceKey] = [.isRegularFileKey]
        var out: [String: URL] = [:]
        // iOS may return /private/var/... for an enumerator even when the
        // Documents URL is /var/... . Resolve both before stripping the prefix.
        let rootPath = root.resolvingSymlinksInPath().path
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: keys,
        ) else { return out }
        for case let url as URL in enumerator {
            guard !url.lastPathComponent.hasPrefix(".") else { continue }
            guard (try? url.resourceValues(forKeys: Set(keys)).isRegularFile) == true else {
                continue
            }
            let filePath = url.resolvingSymlinksInPath().path
            guard filePath.hasPrefix(rootPath + "/") else { continue }
            let path = String(filePath.dropFirst(rootPath.count + 1))
            out[path] = url
        }
        return out
    }

    private func sha(_ url: URL) -> String? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    private func query(_ path: String, expected: String? = nil) -> String {
        var allowed = CharacterSet.urlQueryAllowed
        allowed.remove(charactersIn: "&=+?#")
        func encode(_ value: String) -> String {
            value.addingPercentEncoding(withAllowedCharacters: allowed) ?? value
        }
        var items = ["path=\(encode(path))"]
        if let expected { items.append("expected_revision=\(encode(expected))") }
        return items.joined(separator: "&")
    }

    private func upload(
        base: URL,
        token: String,
        path: String,
        file: URL,
        expected: String? = nil,
    ) async throws -> String {
        var components = URLComponents(
            url: base.appendingPathComponent("api/sync/file"),
            resolvingAgainstBaseURL: false,
        )!
        components.percentEncodedQuery = query(path, expected: expected)
        var request = URLRequest(url: components.url!)
        request.httpMethod = "PUT"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        request.httpBody = try Data(contentsOf: file)
        let (data, response) = try await URLSession.shared.data(for: request)
        try check(response, path: path)
        return try JSONDecoder().decode(RevisionResponse.self, from: data).revision
    }

    private func download(base: URL, token: String, path: String, destination: URL) async throws {
        var components = URLComponents(
            url: base.appendingPathComponent("api/sync/file"),
            resolvingAgainstBaseURL: false,
        )!
        components.percentEncodedQuery = query(path)
        var request = URLRequest(url: components.url!)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let (data, response) = try await URLSession.shared.data(for: request)
        try check(response, path: path)
        try FileManager.default.createDirectory(
            at: destination.deletingLastPathComponent(),
            withIntermediateDirectories: true,
        )
        try data.write(to: destination, options: .atomic)
    }

    private func deleteRemote(base: URL, token: String, path: String, revision: String) async throws {
        var components = URLComponents(
            url: base.appendingPathComponent("api/sync/file"),
            resolvingAgainstBaseURL: false,
        )!
        components.percentEncodedQuery = query(path, expected: revision)
        var request = URLRequest(url: components.url!)
        request.httpMethod = "DELETE"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let (_, response) = try await URLSession.shared.data(for: request)
        try check(response, path: path)
    }

    private func check(_ response: URLResponse, path: String) throws {
        guard let http = response as? HTTPURLResponse else { return }
        if http.statusCode == 401 {
            throw SyncError.server("Token inválido ou expirado")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw SyncError.http(path, http.statusCode)
        }
    }

    private func conflictURL(_ file: URL) -> URL {
        let stamp = ISO8601DateFormatter().string(from: Date()).replacingOccurrences(of: ":", with: "-")
        let stem = file.deletingPathExtension().lastPathComponent
        let ext = file.pathExtension
        return file.deletingLastPathComponent().appendingPathComponent(
            ext.isEmpty ? "\(stem) (conflito iOS \(stamp))" : "\(stem) (conflito iOS \(stamp)).\(ext)",
        )
    }

    private func loadBase() -> [String: String] {
        guard let data = UserDefaults.standard.data(forKey: baseKey),
              let map = try? JSONDecoder().decode([String: String].self, from: data)
        else { return [:] }
        return map
    }

    private func saveBase(_ map: [String: String]) {
        if let data = try? JSONEncoder().encode(map) {
            UserDefaults.standard.set(data, forKey: baseKey)
        }
    }
}
