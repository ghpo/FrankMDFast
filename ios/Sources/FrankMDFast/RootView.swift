import SwiftUI

struct RootView: View {
    @EnvironmentObject private var vault: RustVaultStore
    @AppStorage("themeMode") private var themeMode = "auto"
    @State private var showSettings = false
    @State private var showNotes = false
    @State private var showRename = false
    @State private var renameText = ""
    @State private var previewMode = false
    @State private var preview: RustPreview?
    @Environment(\.colorScheme) private var colorScheme
    @AppStorage("serverURL") private var serverURL = ""
    @AppStorage("syncToken") private var syncToken = ""


    var body: some View {
        NavigationStack {
            Group {
                if let path = vault.currentPath {
                    editor(path: path)
                } else {
                    emptyState
                }
            }
            .navigationTitle(vault.currentPath.map { ($0 as NSString).lastPathComponent } ?? "FrankMDFast")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        showNotes = true
                    } label: {
                        Image(systemName: "sidebar.left")
                    }
                    .accessibilityLabel("Abrir notas")
                }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    if vault.currentPath != nil {
                        Button {
                            let path = vault.currentPath!
                            vault.save()
                            if !previewMode {
                                preview = vault.preview(path)
                                if preview == nil {
                                    vault.status = "Preview indisponível"
                                }
                            }
                            previewMode.toggle()
                        } label: {
                            Image(systemName: previewMode ? "pencil" : "eye")
                        }
                    }
                    Button { vault.create() } label: {
                        Image(systemName: "plus")
                    }
                    if let path = vault.currentPath {
                        Menu {
                            Button("Renomear nota") {
                                renameText = (path as NSString).lastPathComponent.replacingOccurrences(of: ".md", with: "")
                                showRename = true
                            }
                            if let note = vault.notes.first(where: { $0.path == path }) {
                                Button(note.encrypted ? "Descriptografar nota" : "Criptografar nota") {
                                    if note.locked {
                                        vault.status = "Destrave a criptografia em Configurações"
                                    } else if !vault.setEncrypted(path, !note.encrypted) {
                                        vault.status = "Falha ao alterar a criptografia"
                                    }
                                }
                            }
                            Button("Apagar nota", role: .destructive) {
                                vault.delete(path)
                            }
                        } label: {
                            Image(systemName: "ellipsis.circle")
                        }
                    }
                    Button { showSettings = true } label: {
                        Image(systemName: "gearshape")
                    }
                }
            }
            .sheet(isPresented: $showSettings) {
                SettingsView()
                    .environmentObject(vault)
            }
            .sheet(isPresented: $showNotes) {
                NotesBrowserView()
                    .environmentObject(vault)
                    .presentationDetents([.medium, .large])
            }
            .alert("Renomear nota", isPresented: $showRename) {
                TextField("Nome", text: $renameText)
                Button("Cancelar", role: .cancel) {}
                Button("Renomear") {
                    if let path = vault.currentPath { vault.rename(path, to: renameText) }
                }
            }
            .safeAreaInset(edge: .bottom) {
                if vault.currentPath != nil {
                    HStack {
                        Button("Salvar") { vault.save() }
                            .buttonStyle(.borderedProminent)
                        Spacer()
                        Text(vault.status)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal)
                    .padding(.vertical, 8)
                    .background(.bar)
                }
            }
            .preferredColorScheme(preferredScheme)
            .onChange(of: vault.currentPath) { _, _ in
                previewMode = false
                preview = nil
            }
            .task {
                guard !syncToken.isEmpty else { return }
                if (try? await SyncManager.shared.sync(
                    vault: vault,
                    serverURL: serverURL,
                    token: syncToken,
                )) != nil {
                    vault.reload(select: vault.currentPath)
                }
            }
        }
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label("Nenhuma nota aberta", systemImage: "doc.text")
        } description: {
            Text("Escolha uma nota ou crie uma nova.")
        } actions: {
            Button("Nova nota") { vault.create() }
                .buttonStyle(.borderedProminent)
        }
    }

    private func editor(path: String) -> some View {
        Group {
            if previewMode, let preview {
                MarkdownWebView(
                    preview: preview,
                    root: vault.root,
                    dark: colorScheme == .dark,
                )
            } else {
                TextEditor(text: Binding(
                    get: { vault.text },
                    set: { vault.updateText($0) }
                ))
                .font(.system(.body, design: .monospaced))
                .padding()
                .scrollContentBackground(.hidden)
                .background(Color(.systemBackground))
            }
        }
    }

    private var preferredScheme: ColorScheme? {
        switch themeMode {
        case "light": .light
        case "dark": .dark
        default: nil
        }
    }
}
