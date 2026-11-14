import SwiftUI

enum NotesTheme {
    static let accent = Color(uiColor: UIColor { traits in
        traits.userInterfaceStyle == .dark
            ? UIColor(red: 0.90, green: 0.77, blue: 0.28, alpha: 1)
            : UIColor(red: 0.84, green: 0.66, blue: 0.00, alpha: 1)
    })

    static let folderTint = accent
    static let editorBackground = Color(uiColor: .systemBackground)
    static let listBackground = Color(uiColor: .systemGroupedBackground)
}
