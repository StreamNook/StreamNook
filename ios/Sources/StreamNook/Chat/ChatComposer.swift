import SNKit
import SwiftUI

/// The composer under a chat list. Built out in full in the composer pass.
struct ChatComposer: View {
    let session: ChatSession
    @State private var draft = ""

    var body: some View {
        HStack(spacing: 6) {
            TextField(session.modes?.prompt ?? "Send a message", text: $draft)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .glassEffect(in: .rect(cornerRadius: 12))
        }
        .padding(.horizontal, 10)
        .padding(.top, 8)
        .padding(.bottom, 10)
    }
}
