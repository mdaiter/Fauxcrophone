import SwiftUI

struct DeviceView: View {
    @ObservedObject var viewModel: LoopbackViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            Divider()
            metersSection
            Divider()
            sourcesSection
            Divider()
            controlsSection
            Spacer()
        }
        .padding(24)
    }

    private var header: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text("Loopback Device")
                    .font(.title)
                    .bold()
                Text(viewModel.deviceStatusText)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 8) {
                infoRow(label: "Sample Rate", value: "\(Int(viewModel.sampleRate)) Hz")
                infoRow(label: "Buffer Size", value: "\(viewModel.bufferFrames) frames")
                infoRow(label: "Latency", value: String(format: "%.2f ms", viewModel.latencyMs))
            }
        }
    }

    private func infoRow(label: String, value: String) -> some View {
        HStack {
            Text(label)
                .font(.callout)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.callout)
        }
    }

    private var metersSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Levels")
                .font(.headline)
            ForEach(viewModel.levels.indices, id: \.self) { index in
                MeterView(level: viewModel.levels[index])
                    .overlay(alignment: .leading) {
                        Text(viewModel.levelLabels[index])
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .padding(.leading, 4)
                    }
            }
        }
    }

    private var sourcesSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Sources")
                .font(.headline)
            ForEach(Array(viewModel.sources.enumerated()), id: \.element.id) { index, source in
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Toggle(isOn: Binding(
                            get: { viewModel.sources[safe: index]?.isEnabled ?? false },
                            set: { newValue in viewModel.setSourceEnabled(source.id, enabled: newValue) }
                        )) {
                            Text(source.name)
                                .font(.subheadline)
                        }
                        Spacer()
                        Toggle(isOn: Binding(
                            get: { viewModel.sources[safe: index]?.isMuted ?? false },
                            set: { newValue in viewModel.setMute(for: source.id, muted: newValue) }
                        )) {
                            Text("Mute")
                        }
                        .toggleStyle(.switch)
                        .disabled(!source.isEnabled)
                    }

                    HStack {
                        Text(String(format: "Gain: %.1f dB", source.gainDb))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Slider(
                            value: Binding(
                                get: { viewModel.sources[safe: index]?.gainDb ?? 0 },
                                set: { newValue in viewModel.setGain(for: source.id, gainDb: newValue) }
                            ),
                            in: -60...12,
                            step: 0.5
                        )
                        .disabled(!source.isEnabled)
                    }
                }
                Divider()
            }
        }
    }

    private var controlsSection: some View {
        HStack(spacing: 16) {
            Button(viewModel.isDriverRunning ? "Stop Driver" : "Start Driver") {
                viewModel.toggleDriver()
            }
            .buttonStyle(.borderedProminent)

            Button(viewModel.isEngineRunning ? "Stop Engine" : "Start Engine") {
                viewModel.toggleEngine()
            }
            .buttonStyle(.bordered)

            Spacer()

            Button("Save Settings") {
                viewModel.persistSettings()
            }
        }
    }
}

struct DeviceView_Previews: PreviewProvider {
    static var previews: some View {
        DeviceView(viewModel: LoopbackViewModel.preview())
            .frame(width: 680, height: 480)
    }
}

private extension Array {
    subscript(safe index: Index) -> Element? {
        guard indices.contains(index) else { return nil }
        return self[index]
    }
}
