//
//  LoopbackDevice.swift
//  AudioServerPlugIn
//
//  Defines the virtual Core Audio device that exposes a stereo loopback pair.
//

import AudioServerPlugIn
import AVFoundation
import os.log

// MARK: - Constants

private let loopbackManufacturer = "DeviceKit"
private let loopbackDeviceUID = "com.devicekit.loopback.device"
private let loopbackInputUID = "com.devicekit.loopback.input"
private let loopbackOutputUID = "com.devicekit.loopback.output"
private let defaultSampleRate: Double = 48_000.0
private let channelCount = 2

// MARK: - LoopbackDeviceConfig

public struct LoopbackDeviceConfig: Sendable {
    public let sampleRate: Double
    public let bufferFrames: UInt32
    public let channels: UInt32
    public let deviceUID: String
    public let outputStreamUID: String
    public let inputStreamUID: String
    public init(
        sampleRate: Double = defaultSampleRate,
        bufferFrames: UInt32,
        channels: UInt32 = UInt32(channelCount),
        deviceUID: String = loopbackDeviceUID,
        outputStreamUID: String = loopbackOutputUID,
        inputStreamUID: String = loopbackInputUID
    ) {
        self.sampleRate = sampleRate
        self.bufferFrames = bufferFrames
        self.channels = channels
        self.deviceUID = deviceUID
        self.outputStreamUID = outputStreamUID
        self.inputStreamUID = inputStreamUID
    }
}

// MARK: - LoopbackDevice

final class LoopbackDevice: NSObject {
    private let log = Logger(subsystem: loopbackDeviceUID, category: "LoopbackDevice")
    private let deviceID: AudioObjectID
    private let configuration: LoopbackDeviceConfig
    private var mixer: LoopbackMixer?
    private var micEngine: AVAudioEngine?
    private var micMixerNode: AVAudioMixerNode?

    init(deviceID: AudioObjectID, configuration: LoopbackDeviceConfig) {
        self.deviceID = deviceID
        self.configuration = configuration
        super.init()
    }

    func start() throws {
        log.debug("Starting loopback device")
        guard device_kit_start_driver() else {
            throw LoopbackError.rustBridgeFailure
        }
        mixer = try LoopbackMixer(sampleRate: configuration.sampleRate, maxFrames: configuration.bufferFrames)
        try setupMicEngine()
        _ = device_kit_start_engine()
    }

    func stop() {
        log.debug("Stopping loopback device")
        micEngine?.stop()
        micEngine = nil
        micMixerNode = nil
        mixer?.shutdown()
        mixer = nil
        device_kit_stop_engine()
        device_kit_stop_driver()
    }

    private func setupMicEngine() throws {
        let engine = AVAudioEngine()
        let inputNode = engine.inputNode
        let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: configuration.sampleRate, channels: AVAudioChannelCount(channelCount), interleaved: true)
        guard let format else {
            throw LoopbackError.invalidFormat
        }

        let mixerNode = AVAudioMixerNode()
        engine.attach(mixerNode)
        engine.connect(inputNode, to: mixerNode, format: format)

        mixerNode.installTap(onBus: 0, bufferSize: configuration.bufferFrames, format: format) { [weak self] (buffer, _) in
            guard let self, let mixer = self.mixer else { return }
            buffer.frameLength = min(buffer.frameLength, self.configuration.bufferFrames)
            mixer.feedMicInput(buffer: buffer)
        }

        try engine.start()
        micEngine = engine
        micMixerNode = mixerNode
    }

    func processOutput(ioData: UnsafeMutablePointer<AudioBufferList>, frames: UInt32, timestamp: UnsafePointer<AudioTimeStamp>) -> OSStatus {
        guard let mixer else { return kAudioHardwareUnspecifiedError }
        do {
            try mixer.process(ioData: ioData, frames: frames, timestamp: timestamp)
            return noErr
        } catch {
            log.error("Mixer process failed: \(String(describing: error))")
            return kAudioHardwareUnspecifiedError
        }
    }

    func processInput(ioData: UnsafeMutablePointer<AudioBufferList>, frames: UInt32, timestamp: UnsafePointer<AudioTimeStamp>) -> OSStatus {
        // For loopback input we expose the same buffer the output produced.
        return processOutput(ioData: ioData, frames: frames, timestamp: timestamp)
    }

    func setGain(_ gain: Float, forSource index: UInt32) {
        mixer?.setGain(gain, sourceIndex: index)
    }

    func setMute(_ mute: Bool, forSource index: UInt32) {
        mixer?.setMute(mute, sourceIndex: index)
    }
}

// MARK: - Errors

enum LoopbackError: Error {
    case rustBridgeFailure
    case invalidFormat
}
