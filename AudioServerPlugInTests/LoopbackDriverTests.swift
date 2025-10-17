import XCTest

final class LoopbackDriverTests: XCTestCase {
    func testRustBridgeResponds() {
        let sampleRate = device_kit_current_sample_rate()
        XCTAssertGreaterThanOrEqual(sampleRate, 0)

        let frames = device_kit_buffer_size_frames()
        XCTAssertGreaterThanOrEqual(frames, 0)

        let latency = device_kit_latency_ms()
        XCTAssertGreaterThanOrEqual(latency, 0)
    }

    func testLogDrain() {
        _ = device_kit_start_driver()
        if let pointer = device_kit_pop_log() {
            let message = String(cString: pointer)
            XCTAssertFalse(message.isEmpty)
        }
    }
}
