import SNKit
import SwiftUI

/// The app's root: the boot loader, then the first-run wizard when setup is not done, then the
/// tabs. The toasts and the Twitch sign-in sheet sit above all three.
struct AppRoot: View {
    @Bindable var model: AppModel

    var body: some View {
        ZStack {
            ShellBackground()
            switch model.phase {
            case .booting: BootView()
            case .setup: SetupWizard(model: model)
            case .main: MainShell(model: model)
            }
        }
        .background(ToastWindowProbe(toasts: model.toasts))
        .sheet(item: signInPage, onDismiss: { model.signIn.sheetClosed() }) { page in
            SignInSheet(signIn: model.signIn, page: page)
        }
        .sheet(isPresented: webSheetShown, onDismiss: { model.host.sheetDismissed() }) {
            if let request = model.host.sheet {
                WebSheet(host: model.host, request: request)
            }
        }
        .onChange(of: model.signIn.error) { _, error in
            if let error { model.toasts.show(error) }
        }
        .preferredColorScheme(model.look.palette.lightContent ? .dark : .light)
        .tint(SN.accent)
        .font(.snBody)
        .foregroundStyle(SN.textPrimary)
        .snMotionScope()
    }

    private var signInPage: Binding<SignIn.Page?> {
        Binding(get: { model.signIn.page }, set: { model.signIn.page = $0 })
    }

    /// The core's web sheet is up. Setting it false (a swipe down) is the dismissal, which
    /// `sheetDismissed` reports.
    private var webSheetShown: Binding<Bool> {
        Binding(get: { model.host.sheet != nil }, set: { shown in if !shown { model.host.sheetDismissed() } })
    }
}

/// While the app boots: the Penrose mark marching over "Loading StreamNook", nothing else.
struct BootView: View {
    var body: some View {
        VStack(spacing: 24) {
            PenroseMark(width: 56)
            Text("Loading StreamNook")
                .font(.sn(14, 500))
                .foregroundStyle(SN.textSecondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Signed out of Twitch: the app's name, why to sign in, and the one button that does.
struct SignedOutLanding: View {
    let model: AppModel
    /// Shown instead of the usual line when StreamNook signed the person out itself.
    var reason: String?

    private var signIn: SignIn { model.signIn }
    private var shownReason: String? { reason ?? signIn.reason }

    var body: some View {
        VStack(spacing: 0) {
            Text("StreamNook")
                .font(.snHero)
                .foregroundStyle(SN.textPrimary)
                .padding(.bottom, 8)
            Text(shownReason ?? "Watch Twitch with your emotes, badges, and cosmetics everywhere.")
                .font(.snBody)
                .foregroundStyle(SN.textSecondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 280)
                .padding(.bottom, 32)
            SignInButton(model: model, again: shownReason != nil)
        }
        .padding(.horizontal, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// "Sign in with Twitch": Twitch's purple with its glyph, full width up to 320 points, the same
/// button everywhere the app asks for the sign-in. It opens Twitch's own sign-in pages in a sheet.
struct SignInButton: View {
    let model: AppModel
    var again = false
    /// Inside a panel it spans the panel; on its own it stops at 320 points.
    var maxWidth: CGFloat = 320

    private var busy: Bool { model.signIn.busy }

    var body: some View {
        Button {
            Task { await model.signIn.begin(core: model.core) }
        } label: {
            HStack(spacing: 10) {
                if busy {
                    ProgressView().tint(.white)
                } else {
                    PlatformMark(platform: .twitch, size: 18, color: .white)
                }
                Text(busy ? "Waiting for sign-in" : again ? "Sign in again" : "Sign in with Twitch")
            }
        }
        .buttonStyle(.snBrand(SN.twitch))
        .frame(maxWidth: maxWidth)
        .disabled(busy || !model.started)
    }
}

/// A place that needs a Twitch sign-in: the line saying what it gets you, and the button.
struct SignInNote: View {
    let model: AppModel
    let text: String

    var body: some View {
        VStack(spacing: 16) {
            Text(text)
                .font(.snBody)
                .foregroundStyle(SN.textSecondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 300)
            SignInButton(model: model)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 56)
        .padding(.horizontal, 24)
    }
}
