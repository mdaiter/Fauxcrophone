import SwiftUI

@main
struct LoopbackControlApp: App {
    @StateObject private var viewModel = LoopbackViewModel()

    var body: some Scene {
        WindowGroup {
            DeviceView(viewModel: viewModel)
                .frame(minWidth: 620, minHeight: 420)
                .onAppear {
                    viewModel.start()
                }
                .onDisappear {
                    viewModel.stop()
                }
        }
        .commands {
            CommandGroup(after: .appSettings) {
                Button("Reset Meters") {
                    viewModel.resetMeters()
                }
                    .keyboardShortcut("r", modifiers: [.command, .shift])
            }
        }
    }
}
