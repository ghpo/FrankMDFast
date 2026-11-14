import SwiftUI

struct NoteDatesView: View {
    let created: Date
    let modified: Date
    var compact = false

    private var createdText: String {
        guard created != .distantPast else { return "—" }
        return created.formatted(date: .abbreviated, time: .shortened)
    }

    private var modifiedText: String {
        guard modified != .distantPast else { return "—" }
        return modified.formatted(date: .abbreviated, time: .shortened)
    }

    var body: some View {
        Group {
            if compact {
                Text("Criada \(createdText) · Editada \(modifiedText)")
            } else {
                VStack(alignment: .leading, spacing: 1) {
                    Text("Criada \(createdText)")
                    Text("Editada \(modifiedText)")
                }
            }
        }
        .font(.caption2)
        .foregroundStyle(.tertiary)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Criada em \(createdText), editada em \(modifiedText)")
    }
}
