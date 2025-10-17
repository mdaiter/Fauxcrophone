import Foundation
import AudioToolbox
import AVFoundation

public final class LoopbackMixer {
    private var handle: LoopbackMixerHandle?
    private let sampleRate: Double
    private let maxFrames: UInt32

    public init(sampleRate: Double, maxFrames: UInt32) throws {
        self.sampleRate = sampleRate
        self.maxFrames = maxFrames
        guard let handle = loopback_mixer_create(sampleRate, maxFrames) else {
            throw LoopbackError.rustBridgeFailure
        }
        self.handle = handle
    }

    deinit {
        shutdown()
    }

    public func shutdown() {
        if let handle {
            loopback_mixer_destroy(handle)
            self.handle = nil
        }
    }

    public func process(ioData: UnsafeMutablePointer<AudioBufferList>, frames: UInt32, timestamp: UnsafePointer<AudioTimeStamp>) throws {
        guard let handle else { throw LoopbackError.rustBridgeFailure }
        var args = LoopbackRenderArgs(bufferList: ioData, frameCount: frames, timestamp: timestamp)
        let status = loopback_mixer_process(handle, &args)
        guard status == noErr else {
            throw LoopbackError.rustBridgeFailure
        }
    }

    public func feedMicInput(buffer: AVAudioPCMBuffer) {
        guard let handle, let channelData = buffer.floatChannelData else { return }
        let frameLength = Int(buffer.frameLength)
        let channels = Int(buffer.format.channelCount)
        guard channels == 2 else { return }
        let interleaved = UnsafeBufferPointer(start: channelData[0], count: frameLength * channels)
        loopback_mixer_submit_input(handle, interleaved.baseAddress, UInt32(frameLength))
    }

    public func setGain(_ gain: Float, sourceIndex: UInt32) {
        guard let handle else { return }
        loopback_mixer_set_gain(handle, sourceIndex, gain)
    }

    public func setMute(_ mute: Bool, sourceIndex: UInt32) {
        guard let handle else { return }
        loopback_mixer_set_mute(handle, sourceIndex, mute)
    }
}
