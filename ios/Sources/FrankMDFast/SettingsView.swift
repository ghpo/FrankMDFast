import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var vault: RustVaultStore
    @AppStorage("themeMode") private var themeMode = "auto"
    @AppStorage("serverURL") private var serverURL = ""
    @AppStorage("syncToken") private var token = ""
    @State private var password = ""
    @State private var sealPassword = ""
    @State private var sealPasswordConfirm = ""
    @State private var sealError = ""
    @State private var showCreateSeal = false
    @State private var showSealPicker = false
    @State private var syncing = false
    @State private var syncDone = 0
    @State private var syncTotal = 0
    @State private var syncStatus = ""

    private var crypto: (configured: Bool, unlocked: Bool) { vault.cryptoStatus() }

    var body: some View {
        NavigationStack {
            Form {
                Section("Aparência") {
                    Picker("Tema", selection: $themeMode) {
                        Text("Automático").tag("auto")
                        Text("Claro").tag("light")
                        Text("Escuro").tag("dark")
                    }
                }
                Section("Sincronização") {
                    TextField("URL do servidor", text: $serverURL)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                    SecureField("Token remoto", text: $token)
                    Button(
                        syncing
                            ? (syncTotal > 0 ? "Sincronizando… \(syncDone)/\(syncTotal)" : "Sincronizando…")
                            : "Sincronizar agora",
                    ) {
                        syncing = true
                        syncDone = 0
                        syncTotal = 0
                        syncStatus = ""
                        Task {
                            do {
                                let result = try await SyncManager.shared.sync(
                                    vault: vault,
                                    serverURL: serverURL,
                                    token: token,
                                    onProgress: { done, total in
                                        syncDone = done
                                        syncTotal = total
                                    },
                                )
                                vault.reload(select: vault.currentPath)
                                syncStatus = "Sync: \(result.downloaded) baixados, \(result.uploaded) enviados, \(result.deleted) apagados, \(result.conflicts) conflitos"
                            } catch {
                                syncStatus = "Falha no sync: \(error.localizedDescription)"
                            }
                            syncing = false
                            syncDone = 0
                            syncTotal = 0
                        }
                    }
                    .disabled(syncing)
                    if !syncStatus.isEmpty {
                        Text(syncStatus).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Section("Criptografia") {
                    if !crypto.configured {
                        Text("Nenhuma criptografia configurada.").foregroundStyle(.secondary)
                        Button("Criar criptografia") {
                            sealPassword = ""
                            sealPasswordConfirm = ""
                            showCreateSeal = true
                        }
                        Button("Importar seal do desktop…") { showSealPicker = true }
                    } else if crypto.unlocked {
                        Label("Criptografia destravada", systemImage: "lock.open")
                            .foregroundStyle(.green)
                        Button("Bloquear criptografia") {
                            vault.lock()
                            password = ""
                            vault.reload(select: vault.currentPath)
                        }
                    } else {
                        Label("Criptografia bloqueada", systemImage: "lock")
                            .foregroundStyle(.orange)
                        SecureField("Senha da criptografia", text: $password)
                            .textContentType(.password)
                        Button("Desbloquear criptografia") {
                            if !vault.unlock(password) {
                                sealError = "Senha incorreta"
                            } else {
                                password = ""
                                sealError = ""
                                vault.reload(select: vault.currentPath)
                            }
                        }
                        Button("Importar outro seal…") { showSealPicker = true }
                    }
                    if !sealError.isEmpty {
                        Text(sealError).foregroundStyle(.red)
                    }
                }
            }
            .alert("Criar criptografia", isPresented: $showCreateSeal) {
                SecureField("Senha (mínimo 8 caracteres)", text: $sealPassword)
                SecureField("Confirmar senha", text: $sealPasswordConfirm)
                Button("Cancelar", role: .cancel) {}
                Button("Criar") {
                    guard sealPassword.count >= 8 else {
                        sealError = "Use pelo menos 8 caracteres"
                        return
                    }
                    guard sealPassword == sealPasswordConfirm else {
                        sealError = "As senhas não coincidem"
                        return
                    }
                    if vault.createSeal(sealPassword) {
                        sealPassword = ""
                        sealPasswordConfirm = ""
                        sealError = ""
                    } else {
                        sealError = "Não foi possível criar a criptografia"
                    }
                }
            } message: {
                Text("A senha não pode ser recuperada. Guarde-a com segurança.")
            }
            .navigationTitle("Configurações")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fechar") { dismiss() }
                }
            }
            .fileImporter(
                isPresented: $showSealPicker,
                allowedContentTypes: [.json, .data],
            ) { result in
                guard case let .success(url) = result else { return }
                let accessed = url.startAccessingSecurityScopedResource()
                defer { if accessed { url.stopAccessingSecurityScopedResource() } }
                do {
                    let json = try String(contentsOf: url, encoding: .utf8)
                    if !vault.importSeal(json) {
                        sealError = "Seal inválido"
                    } else {
                        sealError = ""
                    }
                } catch {
                    sealError = error.localizedDescription
                }
            }
        }
    }
}
