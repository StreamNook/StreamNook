import SNKit
import SwiftUI

/// The first-run wizard: welcome, accounts, drops, done (no notification step until the iPhone
/// sends alerts). A full-screen layer on the theme background under a faint accent wash; pages
/// swipe sideways past 70 points and move with a short rise. Finishing saves `setup_complete`
/// through the core; closing early only keeps it away for this session.
struct SetupWizard: View {
    let model: AppModel
    @State private var page = Page.welcome
    @State private var forward = true
    @State private var drag: CGFloat = 0
    @State private var lastTurn = Date.distantPast
    @Environment(\.snStill) private var still

    enum Page: Int, CaseIterable { case welcome, accounts, drops, done }

    private var signedIn: Bool { model.signedIn }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                if page != .welcome && page != .done {
                    Button { turn(back: true) } label: {
                        Label("Back", systemImage: Symbol.back)
                            .font(.snBody)
                            .foregroundStyle(SN.textSecondary)
                            .frame(minHeight: 44)
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
            }
            .frame(height: 44)
            .padding(.horizontal, 16)

            ZStack {
                pageView(page)
                    .id(page)
                    .transition(still ? .opacity : .asymmetric(
                        insertion: .opacity.combined(with: .offset(y: 14)),
                        removal: .opacity.combined(with: .offset(y: -14))))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .offset(x: drag)
            .contentShape(Rectangle())
            .gesture(swipe)

            footer
        }
        .background { Wash() }
        .onChange(of: model.nav.setupPage, initial: true) { _, asked in
            if let asked { go(to: asked) }
        }
        .onChange(of: signedIn) { _, now in
            guard now, page == .accounts else { return }
            Task {
                try? await Task.sleep(for: .milliseconds(500))
                turn(back: false)
            }
        }
    }

    // MARK: Pages

    @ViewBuilder
    private func pageView(_ page: Page) -> some View {
        switch page {
        case .welcome:
            PageBody(title: "Welcome to\nStreamNook", titleSize: 44,
                     text: "Yeah yeah, another setup wizard. A few clicks and we'll get out of your way, promise.") {
                PenroseMark(width: 72)
            }
        case .accounts:
            if signedIn {
                PageBody(title: "You're signed in", text: "Your follows, chat, and channel actions are connected.") {
                    Image(systemName: Symbol.checkCircle).font(.system(size: 52)).foregroundStyle(SN.success)
                }
            } else {
                PageBody(title: "Connect your accounts",
                         text: "Connect the platforms you watch on to see your follows, chat, and use channel features.") {
                    PlatformMark(platform: .twitch, size: 56)
                }
            }
        case .drops:
            PageBody(title: "Drops and inventory",
                     text: "Sign in to track Twitch Drops, watch your inventory, and auto-claim rewards.") {
                Image(systemName: Symbol.gift).font(.system(size: 48)).foregroundStyle(SN.accent)
            } extra: {
                Button("Sign in for Drops") {}
                    .buttonStyle(.snSecondary)
                    .frame(maxWidth: 320)
                    .disabled(true)
            }
        case .done:
            PageBody(title: "You're all set", text: "StreamNook is ready. Pick a stream and dive in.") {
                PenroseMark(width: 64)
            } extra: {
                VStack(spacing: 0) {
                    summary("Drops sign-in", done: false)
                    Divider().overlay(SN.borderSubtle)
                    summary("Sign-in", mark: .twitch, done: signedIn)
                    Divider().overlay(SN.borderSubtle)
                    summary("Other platforms", done: false)
                }
                .snPanel()
                .frame(maxWidth: 320)
            }
        }
    }

    private func summary(_ title: String, mark: PlatformMark.Platform? = nil, done: Bool) -> some View {
        HStack(spacing: 8) {
            if let mark { PlatformMark(platform: mark, size: 15) }
            Text(title).font(.snRowTitle).foregroundStyle(SN.textPrimary)
            Spacer()
            if done {
                Image(systemName: Symbol.checkCircle).foregroundStyle(SN.success)
            } else {
                Text("skipped").font(.snRowDetail).foregroundStyle(SN.textMuted)
            }
        }
        .padding(.horizontal, 14)
        .frame(height: 46)
    }

    // MARK: Footer

    private var footer: some View {
        VStack(spacing: 18) {
            HStack(spacing: 6) {
                ForEach(Page.allCases, id: \.self) { step in
                    let past = step.rawValue < page.rawValue
                    Capsule()
                        .fill(step == page ? SN.accent : past ? SN.accent.opacity(0.5) : SN.borderSubtle)
                        .frame(width: step == page ? 40 : 8, height: 8)
                        .shadow(color: step == page ? SN.accent.opacity(0.5) : .clear, radius: 4)
                        .onTapGesture { if past { go(to: step) } }
                        .accessibilityLabel("Step \(step.rawValue + 1)")
                }
            }
            primary
        }
        .padding(.horizontal, 24)
        .padding(.bottom, 20)
    }

    @ViewBuilder
    private var primary: some View {
        if page == .accounts && !signedIn {
            Button { Task { await model.signIn.begin(core: model.core) } } label: {
                HStack(spacing: 10) {
                    if model.signIn.busy {
                        ProgressView().tint(.white)
                        Text("Waiting for sign-in")
                    } else {
                        PlatformMark(platform: .twitch, size: 18, color: .white)
                        Text("Sign in with Twitch")
                    }
                }
            }
            .buttonStyle(.snBrand(SN.twitch))
            .disabled(model.signIn.busy || !model.started)
        } else {
            // The way on is the one filled button on the page; a page's own action above it is
            // glass. Both are the same height, so the step dots never jump between pages.
            Button(page == .welcome ? "Get started" : page == .done ? "Start watching" : "Continue") {
                if page == .done {
                    Task { await model.finishSetup() }
                } else {
                    turn(back: false)
                }
            }
            .buttonStyle(.snPrimary)
        }
    }

    // MARK: Moving between pages

    /// Forward is never allowed off the last page (finishing is a deliberate tap) or past
    /// accounts while Twitch is signed out; back is offered between the first and last pages.
    private func canGo(back: Bool) -> Bool {
        if back { return page != .welcome && page != .done }
        if page == .done { return false }
        if page == .accounts && !signedIn { return false }
        return true
    }

    private func turn(back: Bool) {
        guard canGo(back: back), let next = Page(rawValue: page.rawValue + (back ? -1 : 1)) else { return }
        go(to: next)
    }

    private func go(to next: Page) {
        forward = next.rawValue > page.rawValue
        lastTurn = .now
        withAnimation(SNMotion.run(SNMotion.page)) { page = next }
    }

    /// A sideways drag anywhere on the page, rubber-banded; past 70 points it turns the page. A
    /// second swipe within 400 ms of a turn is ignored.
    private var swipe: some Gesture {
        DragGesture(minimumDistance: 12)
            .onChanged { value in
                guard abs(value.translation.width) > abs(value.translation.height) else { return }
                drag = value.translation.width * 0.15
            }
            .onEnded { value in
                let dx = value.translation.width
                withAnimation(SNMotion.run(SNMotion.settleBack)) { drag = 0 }
                guard Date.now.timeIntervalSince(lastTurn) > 0.4, abs(dx) > 70,
                      abs(dx) > abs(value.translation.height) else { return }
                turn(back: dx > 0)
            }
    }

}

/// A wizard page: the picture, the title, the line, and anything the page adds under them.
private struct PageBody<Art: View, Extra: View>: View {
    let title: String
    var titleSize: CGFloat = 30
    let text: String
    @ViewBuilder let art: Art
    @ViewBuilder let extra: Extra

    init(title: String, titleSize: CGFloat = 30, text: String, @ViewBuilder art: () -> Art,
         @ViewBuilder extra: () -> Extra = { EmptyView() }) {
        self.title = title
        self.titleSize = titleSize
        self.text = text
        self.art = art()
        self.extra = extra()
    }

    var body: some View {
        // The title and its line are one thought, so they sit close; the picture and the page's
        // action stand further off.
        VStack(spacing: 0) {
            art.frame(height: 84)
                .padding(.bottom, 24)
            Text(title)
                .font(.sn(titleSize, 600))
                .foregroundStyle(SN.textPrimary)
                .multilineTextAlignment(.center)
                .lineSpacing(-2)
                .padding(.bottom, 10)
            Text(text)
                .font(.sn(15))
                .foregroundStyle(SN.textSecondary)
                .multilineTextAlignment(.center)
                .lineSpacing(2)
                .frame(maxWidth: 320)
                .fixedSize(horizontal: false, vertical: true)
            extra.padding(.top, 28)
        }
        .padding(.horizontal, 28)
    }
}

/// The wizard's background: the theme background under a faint accent wash from the top.
private struct Wash: View {
    var body: some View {
        ZStack {
            SN.background
            ThemeLightLayer(surface: .shell)
            RadialGradient(colors: [SN.accent.opacity(0.16), .clear], center: .top, startRadius: 0, endRadius: 520)
            RadialGradient(colors: [SN.accent.opacity(0.06), .clear], center: .bottomTrailing, startRadius: 0, endRadius: 420)
        }
        .ignoresSafeArea()
    }
}
