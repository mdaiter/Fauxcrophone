import Foundation
import Combine

struct LoopbackSource: Identifiable {
    let id: UInt32
    var name: String
    var isEnabled: Bool
    var gainDb: Double
    var isMuted: Bool
}

final class LoopbackViewModel: ObservableObject {
    @Published var sampleRate: Double = 48_000
    @Published var bufferFrames: UInt32 = 256
    @Published var latencyMs: Double = 0
    @Published var levels: [Double] = []
    @Published var levelLabels: [String] = []
    @Published var sources: [LoopbackSource] = []
    @Published var isDriverRunning: Bool = false
    @Published var isEngineRunning: Bool = false

    private let bridge = LoopbackBridge.shared
    private var meterCancellable: AnyCancellable?
    private var infoCancellable: AnyCancellable?
    private let defaults = UserDefaults.standard
    private let settingsKey = "LoopbackControlSettings"

    func start() {
        refreshDeviceInfo()
        rebuildSources()
        applyPersistedSettings()

        meterCancellable = Timer
            .publish(every: 0.05, on: .main, in: .common)
            .autoconnect()
            .sink { [weak self] _ in
                self?.updateMeters()
            }

        infoCancellable = Timer
            .publish(every: 2.0, on: .main, in: .common)
            .autoconnect()
            .sink { [weak self] _ in
                self?.refreshDeviceInfo()
            }
    }

    func stop() {
        meterCancellable?.cancel()
        infoCancellable?.cancel()
        meterCancellable = nil
        infoCancellable = nil
    }

    func resetMeters() {
        levels = Array(repeating: 0, count: levelLabels.count)
    }

    func toggleDriver() {
        if isDriverRunning {
            bridge.stopDriver()
            isDriverRunning = false
        } else {
            isDriverRunning = bridge.startDriver()
        }
        persistSettings()
    }

    func toggleEngine() {
        if isEngineRunning {
            bridge.stopEngine()
            isEngineRunning = false
        } else {
            isEngineRunning = bridge.startEngine()
        }
        persistSettings()
    }

    func setGain(for sourceID: UInt32, gainDb: Double) {
        guard let index = sources.firstIndex(where: { $0.id == sourceID }) else { return }
        sources[index].gainDb = gainDb
        _ = bridge.setGain(sourceID, gainDb: gainDb)
        persistSettings()
    }

    func setMute(for sourceID: UInt32, muted: Bool) {
        guard let index = sources.firstIndex(where: { $0.id == sourceID }) else { return }
        sources[index].isMuted = muted
        _ = bridge.setMute(sourceID, muted: muted)
        persistSettings()
    }

    func setSourceEnabled(_ sourceID: UInt32, enabled: Bool) {
        guard let index = sources.firstIndex(where: { $0.id == sourceID }) else { return }
        sources[index].isEnabled = enabled
        bridge.setSourceEnabled(sourceID, enabled: enabled)
        persistSettings()
    }

    func persistSettings() {
        let payload: [String: Any] = [
            "isDriverRunning": isDriverRunning,
            "isEngineRunning": isEngineRunning,
            "sources": sources.map { [
                "id": Int($0.id),
                "enabled": $0.isEnabled,
                "gain": $0.gainDb,
                "mute": $0.isMuted
            ] }
        ]
        defaults.set(payload, forKey: settingsKey)
    }

    static func preview() -> LoopbackViewModel {
        let model = LoopbackViewModel()
        model.sampleRate = 48_000
        model.bufferFrames = 256
        model.latencyMs = 5.3
        model.levelLabels = ["Input 1", "Input 2", "Output 1", "Output 2"]
        model.levels = [0.1, 0.25, 0.6, 0.85]
        model.sources = [
            LoopbackSource(id: 0, name: "Microphone", isEnabled: true, gainDb: 0, isMuted: false),
            LoopbackSource(id: 1, name: "Node Feed", isEnabled: true, gainDb: -6, isMuted: false)
        ]
        model.isDriverRunning = true
        model.isEngineRunning = true
        return model
    }

    private func updateMeters() {
        let snapshot = bridge.fetchLevels()
        DispatchQueue.main.async {
            self.levelLabels = snapshot.labels
            self.levels = snapshot.values
        }
    }

    private func refreshDeviceInfo() {
        let snapshot = bridge.fetchDeviceSnapshot()
        DispatchQueue.main.async {
            self.sampleRate = snapshot.sampleRate
            self.bufferFrames = snapshot.bufferFrames
            self.latencyMs = snapshot.latencyMs
        }
    }

    private func rebuildSources() {
        let sourceSnapshots = bridge.fetchSources()
        DispatchQueue.main.async {
            if sourceSnapshots.isEmpty {
                self.sources = []
            } else {
                self.sources = sourceSnapshots
            }
            self.applyPersistedSettings()
        }
    }

    private func applyPersistedSettings() {
        guard let payload = defaults.dictionary(forKey: settingsKey) else { return }
        if let driverState = payload["isDriverRunning"] as? Bool {
            if driverState {
                isDriverRunning = bridge.startDriver()
            } else {
                bridge.stopDriver()
                isDriverRunning = false
            }
        }
        if let engineState = payload["isEngineRunning"] as? Bool {
            if engineState {
                isEngineRunning = bridge.startEngine()
            } else {
                bridge.stopEngine()
                isEngineRunning = false
            }
        }
        if let savedSources = payload["sources"] as? [[String: Any]] {
            for saved in savedSources {
                guard let rawId = saved["id"] as? Int else { continue }
                let id = UInt32(clamping: rawId)
                if let index = sources.firstIndex(where: { $0.id == id }) {
                    sources[index].isEnabled = saved["enabled"] as? Bool ?? sources[index].isEnabled
                    sources[index].gainDb = saved["gain"] as? Double ?? sources[index].gainDb
                    sources[index].isMuted = saved["mute"] as? Bool ?? sources[index].isMuted
                    bridge.setSourceEnabled(id, enabled: sources[index].isEnabled)
                    _ = bridge.setGain(id, gainDb: sources[index].gainDb)
                    _ = bridge.setMute(id, muted: sources[index].isMuted)
                }
            }
        }
    }
}
