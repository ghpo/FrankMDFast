import SwiftUI
import UIKit

struct EditorView: View {
    @EnvironmentObject private var vault: RustVaultStore
    @Environment(\.dismiss) private var dismiss
    let path: String

    @State private var previewMode = false
    @State private var preview: RustPreview?
    @State private var showFormatting = false
    @State private var showAttachments = false
    @State private var showRename = false
    @State private var renameText = ""
    @State private var showDeleteConfirm = false
    @State private var showSettings = false
    @Environment(\.colorScheme) private var colorScheme

    private var crypto: (configured: Bool, unlocked: Bool) { vault.cryptoStatus() }

    private var note: NoteEntry? {
        vault.notes.first { $0.path == path || $0.path == vault.currentPath }
    }

    var body: some View {
        VStack(spacing: 0) {
            metadata
            Group {
                if note?.locked == true {
                    ContentUnavailableView(
                        "Nota bloqueada",
                        systemImage: "lock.fill",
                        description: Text("Destrave a criptografia em Configurações para editar esta nota."),
                    )
                } else if previewMode, let preview {
                    MarkdownWebView(preview: preview, root: vault.root, dark: colorScheme == .dark)
                } else {
                    TextField("Título", text: Binding(
                        get: { vault.currentTitle },
                        set: { vault.updateTitle($0) },
                    ))
                    .font(.system(.largeTitle, design: .default).weight(.bold))
                    .textFieldStyle(.plain)
                    .padding(.horizontal, 18)
                    .padding(.top, 6)
                    TextEditor(text: Binding(
                        get: { vault.bodyText },
                        set: { vault.updateBody($0) },
                    ))
                    .font(.system(.body, design: .default))
                    .lineSpacing(4)
                    .padding(.horizontal, 14)
                    .scrollContentBackground(.hidden)
                    .background(NotesTheme.editorBackground)
                }
            }
        }
        .background(Color(.systemBackground))
        .navigationTitle(vault.currentTitle.isEmpty ? "Nota" : vault.currentTitle)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button {
                    let p = path
                    vault.save()
                    if !previewMode {
                        preview = vault.preview(p)
                    }
                    previewMode.toggle()
                } label: {
                    Image(systemName: previewMode ? "pencil" : "eye")
                }
                ShareLink(item: vault.text) {
                    Image(systemName: "square.and.arrow.up")
                }
                Menu {
                    Button(vault.isPinned(path) ? "Desafixar nota" : "Fixar nota") {
                        vault.togglePin(path)
                    }
                    Button("Renomear arquivo") {
                        renameText = (path as NSString).lastPathComponent.replacingOccurrences(of: ".md", with: "")
                        showRename = true
                    }
                    if !crypto.configured {
                        Button("Configurar criptografia…") { showSettings = true }
                    } else if !crypto.unlocked {
                        Button("Destravar criptografia…") { showSettings = true }
                    } else if let note {
                        Button(note.encrypted ? "Descriptografar nota" : "Criptografar nota") {
                            vault.save()
                            _ = vault.setEncrypted(path, !note.encrypted)
                        }
                    }
                    Button("Anexos") { showAttachments = true }
                    Button("Copiar como Markdown") {
                        UIPasteboard.general.string = vault.text
                        vault.status = "Markdown copiado"
                    }
                    Button("Apagar", role: .destructive) { showDeleteConfirm = true }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            editorToolbar
        }
        .onAppear {
            if vault.currentPath != path { vault.open(path) }
        }
        .onDisappear { vault.save() }
        .sheet(isPresented: $showFormatting) {
            FormattingView { insertion in
                vault.updateBody(vault.bodyText + insertion)
                vault.save()
            }
            .presentationDetents([.medium])
        }
        .sheet(isPresented: $showAttachments) {
            NavigationStack {
                AttachmentsView(showsCloseButton: true)
                    .environmentObject(vault)
            }
        }
        .sheet(isPresented: $showSettings) {
            SettingsView()
                .environmentObject(vault)
        }
        .alert("Renomear nota", isPresented: $showRename) {
            TextField("Nome", text: $renameText)
            Button("Cancelar", role: .cancel) {}
            Button("Renomear") { vault.rename(path, to: renameText) }
        }
        .confirmationDialog("Apagar nota?", isPresented: $showDeleteConfirm) {
            Button("Apagar", role: .destructive) {
                vault.delete(path)
                dismiss()
            }
            Button("Cancelar", role: .cancel) {}
        }
    }

    private var metadata: some View {
        HStack(spacing: 8) {
            if let note {
                NoteDatesView(created: note.created, modified: note.modified, compact: true)
            }
            if note?.encrypted == true {
                Image(systemName: note?.locked == true ? "lock.fill" : "lock")
                    .foregroundStyle(note?.locked == true ? .orange : .secondary)
            }
            Spacer()
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 18)
        .padding(.top, 4)
        .accessibilityElement(children: .combine)
    }

    private var editorToolbar: some View {
        HStack(spacing: 8) {
            Button { showFormatting = true } label: {
                Image(systemName: "textformat")
            }
            Button { insert("- [ ] ") } label: {
                Image(systemName: "checklist")
            }
            Button { showAttachments = true } label: {
                Image(systemName: "paperclip")
            }
            Spacer()
            Text(vault.status)
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .font(.title3)
        .padding(.horizontal, 18)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial, in: Capsule())
        .overlay(Capsule().stroke(.separator.opacity(0.35), lineWidth: 0.5))
        .padding(.horizontal, 16)
        .padding(.bottom, 6)
    }

    private func insert(_ text: String) {
        vault.updateBody(vault.bodyText + text)
        vault.save()
    }
}

private struct FormattingView: View {
    @Environment(\.dismiss) private var dismiss
    let onInsert: (String) -> Void

    var body: some View {
        NavigationStack {
            List {
                Section("Texto") {
                    formatButton("Título", "# Título\n")
                    formatButton("Subtítulo", "## Subtítulo\n")
                    formatButton("Negrito", "**texto**")
                    formatButton("Itálico", "_texto_")
                    formatButton("Código", "`código`")
                }
                Section("Listas") {
                    formatButton("Lista", "- item\n")
                    formatButton("Checklist", "- [ ] tarefa\n")
                    formatButton("Numerada", "1. item\n")
                }
                Section("Adicionar") {
                    formatButton("Divisor", "\n---\n")
                    formatButton("Link", "[texto](https://)")
                }
            }
            .navigationTitle("Formatação")
            .toolbar { Button("Fechar") { dismiss() } }
        }
    }

    private func formatButton(_ title: String, _ insertion: String) -> some View {
        Button(title) {
            onInsert(insertion)
            dismiss()
        }
    }
}
