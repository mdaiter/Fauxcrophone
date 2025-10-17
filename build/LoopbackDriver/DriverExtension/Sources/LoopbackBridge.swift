import Foundation

@_silgen_name("loopback_mixer_global_handle")
private func loopback_mixer_global_handle() -> UnsafeMutableRawPointer?

@_silgen_name("loopback_mixer_set_gain")
private func loopback_mixer_set_gain(
    _ handle: UnsafeMutableRawPointer?,
    _ sourceIndex: UInt32,
    _ gainDb: Float
)

@_silgen_name("loopback_mixer_set_mute")
private func loopback_mixer_set_mute(
    _ handle: UnsafeMutableRawPointer?,
    _ sourceIndex: UInt32,
    _ mute: Bool
)

@_silgen_name("device_kit_get_levels")
private func device_kit_get_levels(_ levelsOut: UnsafeMutablePointer<CLoopbackLevels>) -> Bool

@_silgen_name("device_kit_current_sample_rate")
private func device_kit_current_sample_rate() -> Double

@_silgen_name("device_kit_buffer_size_frames")
private func device_kit_buffer_size_frames() -> UInt32

@_silgen_name("device_kit_latency_ms")
private func device_kit_latency_ms() -> Double

@_silgen_name("device_kit_start_driver")
private func device_kit_start_driver() -> Bool

@_silgen_name("device_kit_stop_driver")
private func device_kit_stop_driver()

@_silgen_name("device_kit_start_engine")
private func device_kit_start_engine() -> Bool

@_silgen_name("device_kit_stop_engine")
private func device_kit_stop_engine()

@_silgen_name("device_kit_source_count")
private func device_kit_source_count() -> UInt32

@_silgen_name("device_kit_source_is_enabled")
private func device_kit_source_is_enabled(_ index: UInt32) -> Bool

@_silgen_name("device_kit_set_source_enabled")
private func device_kit_set_source_enabled(_ index: UInt32, _ enabled: Bool)

private let maxMeters = 8

/// Mirror of the C struct exposed through the bridge header.
@frozen
private struct CLoopbackLevels {
    var inputs: (Float, Float, Float, Float, Float, Float, Float, Float)
    var outputs: (Float, Float, Float, Float, Float, Float, Float, Float)
    var input_count: UInt32
    var output_count: UInt32

    init() {
        inputs = (0, 0, 0, 0, 0, 0, 0, 0)
        outputs = (0, 0, 0, 0, 0, 0, 0, 0)
        input_count = 0
        output_count = 0
    }
}

struct LevelSnapshot {
    var labels: [String]
    var values: [Double]
}

struct DeviceSnapshot {
    var sampleRate: Double
    var bufferFrames: UInt32
    var latencyMs: Double
}

struct SourceSnapshot {
    var id: UInt32
    var name: String
    var isEnabled: Bool
    var gainDb: Double
    var isMuted: Bool
}

final class LoopbackBridge {
    static let shared = LoopbackBridge()

    private init() {}

    func fetchLevels() -> LevelSnapshot {
        var raw = CLoopbackLevels()
        guard device_kit_get_levels(&raw) else {
            return LevelSnapshot(labels: [], values: [])
        }
        let inputs = tupleToArray(raw.inputs, count: Int(min(raw.input_count, UInt32(maxMeters))))
        let outputs = tupleToArray(raw.outputs, count: Int(min(raw.output_count, UInt32(maxMeters))))

        var labels: [String] = []
        var values: [Double] = []

        for (idx, value) in inputs.enumerated() {
            labels.append("Input \(idx + 1)")
            values.append(Double(value).clamped())
        }
        for (idx, value) in outputs.enumerated() {
            labels.append("Output \(idx + 1)")
            values.append(Double(value).clamped())
        }

        return LevelSnapshot(labels: labels, values: values)
    }

    func fetchDeviceSnapshot() -> DeviceSnapshot {
        DeviceSnapshot(
            sampleRate: device_kit_current_sample_rate(),
            bufferFrames: device_kit_buffer_size_frames(),
            latencyMs: device_kit_latency_ms()
        )
    }

    func fetchSources() -> [SourceSnapshot] {
        let count = device_kit_source_count()
        guard count > 0 else { return [] }
        return (0..<count).map { index in
            SourceSnapshot(
                id: index,
                name: defaultName(for: index),
                isEnabled: device_kit_source_is_enabled(index),
                gainDb: 0.0,
                isMuted: false
            )
        }
    }

    func setSourceEnabled(_ id: UInt32, enabled: Bool) {
        device_kit_set_source_enabled(id, enabled)
    }

    func setGain(_ id: UInt32, gainDb: Double) -> Bool {
        guard let handle = loopback_mixer_global_handle() else {
            return false
        }
        let amplitude = pow(10.0, gainDb / 20.0)
        loopback_mixer_set_gain(handle, id, Float(amplitude))
        return true
    }

    func setMute(_ id: UInt32, muted: Bool) -> Bool {
        guard let handle = loopback_mixer_global_handle() else {
            return false
        }
        loopback_mixer_set_mute(handle, id, muted)
        return true
    }

    func startDriver() -> Bool {
        device_kit_start_driver()
    }

    func stopDriver() {
        device_kit_stop_driver()
    }

    func startEngine() -> Bool {
        device_kit_start_engine()
    }

    func stopEngine() {
        device_kit_stop_engine()
    }

    private func defaultName(for index: UInt32) -> String {
        switch index {
        case 0: return "Microphone"
        case 1: return "Node Feed"
        default: return "Source #\(index)"
        }
    }

    private func tupleToArray(_ tuple: (Float, Float, Float, Float, Float, Float, Float, Float), count: Int) -> [Float] {
        return withUnsafePointer(to: tuple) { pointer in
            pointer.withMemoryRebound(to: Float.self, capacity: maxMeters) { floatPtr in
                Array(UnsafeBufferPointer(start: floatPtr, count: count))
            }
        }
    }
}

private extension Double {
    func clamped() -> Double {
        if self.isNaN || self.isInfinite { return 0 }
        return min(max(self, 0), 1)
    }
}
