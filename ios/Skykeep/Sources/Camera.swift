import AVFoundation
import CoreMedia
import QuartzCore
import VideoToolbox
import os

/// The camera, encoded for the computer with the least delay the hardware allows.
///
/// - Capture: the chosen lens at 1920×1080, 60 frames a second when it can, late frames dropped
///   by the system rather than queued.
/// - Encoding: the hardware H.264 encoder in its low-latency mode (WWDC21; on iOS 27 the normal
///   mode also ignores the keyframe interval), real time, no frame reordering, so each picture
///   leaves as soon as it's encoded.
/// - Keyframes: every two seconds, and at once when the computer joins or asks after a drop.
final class Camera: NSObject, @unchecked Sendable, AVCaptureVideoDataOutputSampleBufferDelegate {
    struct Lens: Identifiable, Hashable, Sendable {
        let id: String
        let label: String
        let front: Bool
    }

    static let width: Int32 = 1920
    static let height: Int32 = 1080

    let session = AVCaptureSession()
    var onPicture: (@Sendable (Data, Bool) -> Void)?
    /// Each captured frame, for the floating picture-in-picture preview.
    var onFrame: (@Sendable (CMSampleBuffer) -> Void)?
    private(set) var fps: Int32 = 60

    private let sessionQueue = DispatchQueue(label: "camera.session", qos: .userInitiated)
    private let frameQueue = DispatchQueue(label: "camera.frames", qos: .userInteractive)
    private let output = AVCaptureVideoDataOutput()
    private var input: AVCaptureDeviceInput?
    private var encoder: VTCompressionSession?
    private let wantKeyframe = OSAllocatedUnfairLock(initialState: true)
    private var lastKeyframe: CMTime = .zero
    /// Encode only while a computer is connected; the preview alone costs no encoding.
    let sending = OSAllocatedUnfairLock(initialState: false)

    static func lenses() -> [Lens] {
        let back = AVCaptureDevice.DiscoverySession(
            deviceTypes: [.builtInUltraWideCamera, .builtInWideAngleCamera, .builtInTelephotoCamera],
            mediaType: .video, position: .back
        ).devices
        let front = AVCaptureDevice.DiscoverySession(deviceTypes: [.builtInWideAngleCamera], mediaType: .video, position: .front).devices
        var out: [Lens] = back.map { d in
            let label: String
            switch d.deviceType {
            case .builtInUltraWideCamera: label = "0.5×"
            case .builtInTelephotoCamera: label = "Tele"
            default: label = "1×"
            }
            return Lens(id: d.uniqueID, label: label, front: false)
        }
        out += front.map { Lens(id: $0.uniqueID, label: "Front", front: true) }
        return out
    }

    func configure(lens: Lens) {
        sessionQueue.async { [self] in
            guard let device = AVCaptureDevice(uniqueID: lens.id) else { return }
            session.beginConfiguration()
            session.sessionPreset = .inputPriority
            if session.isMultitaskingCameraAccessSupported { session.isMultitaskingCameraAccessEnabled = true }
            if let old = input { session.removeInput(old) }
            if let new = try? AVCaptureDeviceInput(device: device), session.canAddInput(new) {
                session.addInput(new)
                input = new
            }
            choose(format: device)
            if session.outputs.isEmpty {
                output.alwaysDiscardsLateVideoFrames = true
                output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange]
                output.setSampleBufferDelegate(self, queue: frameQueue)
                if session.canAddOutput(output) { session.addOutput(output) }
            }
            if let c = output.connection(with: .video) {
                if c.isVideoRotationAngleSupported(0) { c.videoRotationAngle = 0 }
                if c.isVideoMirroringSupported { c.automaticallyAdjustsVideoMirroring = false; c.isVideoMirrored = lens.front }
            }
            session.commitConfiguration()
            wantKeyframe.withLock { $0 = true }
            if !session.isRunning { session.startRunning() }
        }
    }

    func stop() {
        sessionQueue.async { [self] in session.stopRunning() }
    }

    func requestKeyframe() {
        wantKeyframe.withLock { $0 = true }
    }

    /// 1920×1080 at the highest rate up to 60 the lens offers.
    private func choose(format device: AVCaptureDevice) {
        let candidates = device.formats.filter {
            let d = CMVideoFormatDescriptionGetDimensions($0.formatDescription)
            return d.width == Self.width && d.height == Self.height
                && CMFormatDescriptionGetMediaSubType($0.formatDescription) == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        }
        let best = candidates.max { a, b in
            (a.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0) < (b.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0)
        }
        guard let format = best, (try? device.lockForConfiguration()) != nil else { return }
        device.activeFormat = format
        let top = min(60, format.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 30)
        fps = Int32(top)
        device.activeVideoMinFrameDuration = CMTime(value: 1, timescale: fps)
        device.activeVideoMaxFrameDuration = CMTime(value: 1, timescale: fps)
        device.unlockForConfiguration()
        encoder.map { VTCompressionSessionInvalidate($0) }
        encoder = nil
    }

    private func makeEncoder() -> VTCompressionSession? {
        let spec: [CFString: Any] = [kVTVideoEncoderSpecification_EnableLowLatencyRateControl: true]
        var made: VTCompressionSession?
        var status = VTCompressionSessionCreate(
            allocator: nil, width: Self.width, height: Self.height, codecType: kCMVideoCodecType_H264,
            encoderSpecification: spec as CFDictionary, imageBufferAttributes: nil, compressedDataAllocator: nil,
            outputCallback: nil, refcon: nil, compressionSessionOut: &made
        )
        if status != noErr {
            // An encoder without the low-latency mode is refused outright rather than ignored:
            // fall back to the ordinary one, still in real time.
            status = VTCompressionSessionCreate(
                allocator: nil, width: Self.width, height: Self.height, codecType: kCMVideoCodecType_H264,
                encoderSpecification: nil, imageBufferAttributes: nil, compressedDataAllocator: nil,
                outputCallback: nil, refcon: nil, compressionSessionOut: &made
            )
        }
        guard status == noErr, let s = made else { return nil }
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_RealTime, value: kCFBooleanTrue)
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_ProfileLevel, value: kVTProfileLevel_H264_High_AutoLevel)
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AllowFrameReordering, value: kCFBooleanFalse)
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_ExpectedFrameRate, value: NSNumber(value: fps))
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, value: NSNumber(value: 2))
        // Over the cable there is room to spare: 16 Mbit/s keeps 1080p60 clean.
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AverageBitRate, value: NSNumber(value: 16_000_000))
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality, value: kCFBooleanTrue)
        VTCompressionSessionPrepareToEncodeFrames(s)
        return s
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sample: CMSampleBuffer, from connection: AVCaptureConnection) {
        onFrame?(sample)
        guard sending.withLock({ $0 }), onPicture != nil, let image = CMSampleBufferGetImageBuffer(sample) else { return }
        if encoder == nil { encoder = makeEncoder() }
        guard let encoder else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(sample)
        // The capture moment on the shared clock: now, less how long ago the sensor took it.
        let capturedUs = Wire.nowUs() - Int64((CACurrentMediaTime() - pts.seconds) * 1_000_000)
        var force = wantKeyframe.withLock { v in defer { v = false }; return v }
        if (pts - lastKeyframe).seconds >= 2 { force = true }
        if force { lastKeyframe = pts }
        let options = force ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        VTCompressionSessionEncodeFrame(encoder, imageBuffer: image, presentationTimeStamp: pts, duration: .invalid,
                                        frameProperties: options, infoFlagsOut: nil) { [weak self] status, _, encoded in
            guard status == noErr, let encoded, let self else { return }
            let (unit, key) = Self.annexB(encoded)
            guard !unit.isEmpty else { return }
            self.onPicture?(Wire.packet(.video, flags: key ? Wire.keyframe : 0, capturedUs: capturedUs, body: unit), key)
        }
    }

    /// The encoder's length-prefixed units as Annex B, with the parameter sets in front of
    /// every keyframe so the computer can start decoding there.
    static func annexB(_ sample: CMSampleBuffer) -> (Data, Bool) {
        let startCode: [UInt8] = [0, 0, 0, 1]
        var out = Data()
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
        let key = !(attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool ?? false)
        if key, let format = CMSampleBufferGetFormatDescription(sample) {
            var count = 0
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                                                               parameterSetSizeOut: nil, parameterSetCountOut: &count, nalUnitHeaderLengthOut: nil)
            for i in 0 ..< count {
                var pointer: UnsafePointer<UInt8>?
                var size = 0
                if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: i, parameterSetPointerOut: &pointer,
                                                                      parameterSetSizeOut: &size, parameterSetCountOut: nil,
                                                                      nalUnitHeaderLengthOut: nil) == noErr, let pointer {
                    out.append(contentsOf: startCode)
                    out.append(pointer, count: size)
                }
            }
        }
        guard let block = CMSampleBufferGetDataBuffer(sample) else { return (out, key) }
        let total = CMBlockBufferGetDataLength(block)
        var bytes = [UInt8](repeating: 0, count: total)
        guard CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: total, destination: &bytes) == noErr else { return (out, key) }
        var offset = 0
        while offset + 4 <= total {
            let length = Int(bytes[offset]) << 24 | Int(bytes[offset + 1]) << 16 | Int(bytes[offset + 2]) << 8 | Int(bytes[offset + 3])
            offset += 4
            guard length > 0, offset + length <= total else { break }
            out.append(contentsOf: startCode)
            out.append(contentsOf: bytes[offset ..< offset + length])
            offset += length
        }
        return (out, key)
    }
}
