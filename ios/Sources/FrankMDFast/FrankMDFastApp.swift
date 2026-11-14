import SwiftUI

@main
struct FrankMDFastApp: App {
    @StateObject private var vault = RustVaultStore()

    var body: some Scene {
        WindowGroup {
            FoldersView()
                .environmentObject(vault)
                .tint(NotesTheme.accent)
        }
    }
}
