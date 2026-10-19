import SNKit
import SwiftUI

/// The sleep timer in the player's menu, under the qualities: the presets and Custom while it is
/// off; while it runs, when it stops playback and Off. The core runs the timer and words every
/// label, so the phones read the same.
struct SleepTimerSection: View {
    let model: AppModel
    @State private var customShown = false

    private var timer: SleepTimerState? { model.sleepTimer }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Sleep timer").snSectionStyle()
            if let timer, timer.running {
                HStack(spacing: 10) {
                    Image(systemName: "moon.zzz.fill")
                        .font(.system(size: 15, weight: .medium))
                        .foregroundStyle(SN.accent)
                    Text(timer.rowLabel ?? "")
                        .font(.sn(15))
                        .foregroundStyle(SN.textPrimary)
                        .monospacedDigit()
                    Spacer(minLength: 8)
                    Button("Off") { Task { await model.cancelSleepTimer() } }
                        .font(.sn(14, 600))
                        .foregroundStyle(SN.textSecondary)
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                        .buttonStyle(CardPress())
                }
                .padding(.horizontal, 8)
            } else {
                HStack(spacing: 8) {
                    ForEach(timer?.presets ?? Self.fallbackPresets, id: \.self) { preset in
                        chip(preset.label) { Task { await model.startSleepTimer(minutes: preset.minutes) } }
                    }
                    chip("Custom") { customShown = true }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 4)
        .padding(.bottom, 16)
        // The row counts down while the menu is open.
        .task {
            while !Task.isCancelled {
                await model.refreshSleepTimer()
                try? await Task.sleep(for: .seconds(20))
            }
        }
        .sheet(isPresented: $customShown) {
            SleepTimerPicker { minutes in Task { await model.startSleepTimer(minutes: minutes) } }
        }
    }

    private func chip(_ label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(label)
                .font(.sn(13, 600))
                .foregroundStyle(SN.textPrimary)
                .lineLimit(1)
                .fixedSize()
                .padding(.horizontal, 12)
                .frame(height: 34)
                .snGlass(Capsule(), interactive: true)
        }
        .buttonStyle(.plain)
    }

    /// Shown for the moment before the core answers with its own presets.
    private static let fallbackPresets = [
        SleepTimerState.Preset(minutes: 15, label: "15 min"), SleepTimerState.Preset(minutes: 30, label: "30 min"),
        SleepTimerState.Preset(minutes: 60, label: "1 h"), SleepTimerState.Preset(minutes: 120, label: "2 h"),
    ]
}

/// A custom sleep timer: hours and minutes on two wheels, then Start.
struct SleepTimerPicker: View {
    let onStart: (Int) -> Void
    @State private var hours = 0
    @State private var minutes = 45
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: "Sleep timer")
            HStack(spacing: 0) {
                Picker("Hours", selection: $hours) {
                    ForEach(0...12, id: \.self) { Text("\($0) h").tag($0) }
                }
                .pickerStyle(.wheel)
                Picker("Minutes", selection: $minutes) {
                    ForEach(0...59, id: \.self) { Text("\($0) min").tag($0) }
                }
                .pickerStyle(.wheel)
            }
            .frame(height: 160)
            .padding(.horizontal, 16)
            Button("Start") {
                guard let total = SleepTimerState.customMinutes(hours: hours, minutes: minutes) else { return }
                onStart(total)
                dismiss()
            }
            .buttonStyle(.snPrimary)
            .disabled(SleepTimerState.customMinutes(hours: hours, minutes: minutes) == nil)
            .padding(16)
        }
        .fittedSheet()
    }
}
