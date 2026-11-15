@preconcurrency import AVFoundation

/// The microphone, sent as it is captured: 5 ms blocks of 48 kHz mono, straight from the
/// audio hardware's own callback, so no block waits for a bigger one.
///
/// Apple's voice processing is on, the same as in FaceTime: echo cancellation, noise
/// suppression, and the Voice Isolation and Wide Spectrum choices in Control Center.
final class Microphone: @unchecked Sendable {
    var onSound: (@Sendable (Data) -> Void)?

    private let engine = AVAudioEngine()
    private var sink: AVAudioSinkNode?
    private let target = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 48_000, channels: 1, interleaved: true)!

    func start() throws {
        let session = AVAudioSession.sharedInstance()
        // As FaceTime on speaker, not a call held to the ear: the receiver's near-mouth tuning
        // made speech a metre away quiet.
        try session.setCategory(.playAndRecord, mode: .videoChat, options: [.mixWithOthers, .defaultToSpeaker])
        try session.setPreferredSampleRate(48_000)
        try session.setPreferredIOBufferDuration(0.005)
        try session.setActive(true)

        let input = engine.inputNode
        try? input.setVoiceProcessingEnabled(true)
        let format = input.outputFormat(forBus: 0)
        let converter = AVAudioConverter(from: format, to: target)
        let target = self.target
        let sink = AVAudioSinkNode { [weak self] timestamp, frames, list -> OSStatus in
            guard let self, let onSound = self.onSound else { return noErr }
            let capturedUs = Wire.nowUs() - Int64(Double(frames) / format.sampleRate * 1_000_000)
            let pcm = Self.int16(list, frames: frames, format: format, converter: converter, target: target)
            if !pcm.isEmpty { onSound(Wire.packet(.audio, capturedUs: capturedUs, body: pcm)) }
            _ = timestamp
            return noErr
        }
        engine.attach(sink)
        engine.connect(input, to: sink, format: format)
        self.sink = sink
        engine.prepare()
        try engine.start()
        log.notice("microphone running: \(format.sampleRate, privacy: .public) Hz, \(format.channelCount, privacy: .public) channel(s), \(String(describing: format.commonFormat), privacy: .public), interleaved \(format.isInterleaved, privacy: .public)")
    }

    /// The built-in microphone on the camera's side, pointed outwards (cardioid), as the Camera
    /// app picks it: the person the camera sees is the one speaking.
    func face(_ side: Camera.Side) {
        let session = AVAudioSession.sharedInstance()
        guard let builtIn = session.availableInputs?.first(where: { $0.portType == .builtInMic }),
              let source = builtIn.dataSources?.first(where: { $0.orientation == (side == .front ? .front : .back) })
        else { return }
        if let patterns = source.supportedPolarPatterns, patterns.contains(.cardioid) {
            try? source.setPreferredPolarPattern(.cardioid)
        }
        try? builtIn.setPreferredDataSource(source)
        try? session.setPreferredInput(builtIn)
        log.notice("microphone facing \(side == .front ? "front" : "back", privacy: .public)")
    }

    func stop() {
        engine.stop()
        if let sink { engine.detach(sink) }
        sink = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    /// The first channel as 48 kHz Int16: directly when the hardware already runs at 48 kHz in
    /// float (the usual case), through the converter otherwise.
    private static func int16(_ list: UnsafePointer<AudioBufferList>, frames: AVAudioFrameCount, format: AVAudioFormat,
                              converter: AVAudioConverter?, target: AVAudioFormat) -> Data {
        let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: list))
        guard let first = buffers.first, let raw = first.mData else { return Data() }
        if format.sampleRate == 48_000 && format.commonFormat == .pcmFormatFloat32 {
            let floats = raw.assumingMemoryBound(to: Float.self)
            let stride = format.isInterleaved ? Int(format.channelCount) : 1
            var out = Data(count: Int(frames) * 2)
            out.withUnsafeMutableBytes { dst in
                let samples = dst.bindMemory(to: Int16.self)
                for i in 0 ..< Int(frames) {
                    let v = max(-1, min(1, floats[i * stride]))
                    samples[i] = Int16(v * 32767).littleEndian
                }
            }
            return out
        }
        guard let converter,
              let input = AVAudioPCMBuffer(pcmFormat: format, bufferListNoCopy: list),
              let dest = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: AVAudioFrameCount(Double(frames) * 48_000 / format.sampleRate) + 16)
        else { return Data() }
        // The converter asks for input synchronously, inside convert(), on this thread.
        nonisolated(unsafe) var given = false
        nonisolated(unsafe) let source = input
        _ = converter.convert(to: dest, error: nil) { _, status in
            if given { status.pointee = .noDataNow; return nil }
            given = true
            status.pointee = .haveData
            return source
        }
        guard let channel = dest.int16ChannelData else { return Data() }
        return Data(bytes: channel[0], count: Int(dest.frameLength) * 2)
    }
}
