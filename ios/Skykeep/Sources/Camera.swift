import AVFoundation
import CoreMedia
import QuartzCore
import VideoToolbox
import os

let log = Logger(subsystem: "io.github.yashwanthkamireddi.Skykeep", category: "camera")

/// The camera, encoded for the computer with the least delay the hardware allows.
///
/// - Lenses: the back camera is Apple's combined camera (ultra wide, wide and telephoto as one
///   device), so zooming glides from lens to lens with no break in the picture, as in the
///   Camera app. The front camera is its own device.
/// - Upright: Apple's rotation coordinator says how the phone is held; the picture is turned to
///   match. What goes to the computer is always 1920×1080: a picture taken in portrait is
///   centred in it with black at the sides, since calls and recordings expect one camera shape
///   and lose the camera if it changes.
/// - Never mirrored: webcams send the true picture and each app mirrors only its own preview,
///   so others see writing the right way round. The phone's preview is mirrored, as FaceTime's.
/// - Encoding: the hardware H.264 encoder in its low-latency mode (WWDC21; on iOS 27 the normal
///   mode also ignores the keyframe interval), real time, no frame reordering, so each picture
///   leaves as soon as it's encoded. Keyframes every two seconds, and at once when asked.
final class Camera: NSObject, @unchecked Sendable, AVCaptureVideoDataOutputSampleBufferDelegate {
    enum Side: Sendable { case back, front }

    static let width: Int32 = 1920
    static let height: Int32 = 1080

    let session = AVCaptureSession()
    var onPicture: (@Sendable (Data, Bool) -> Void)?
    /// Each captured frame, for the floating picture-in-picture preview.
    var onFrame: (@Sendable (CMSampleBuffer) -> Void)?
    /// The zoom as the Camera app shows it (0.5×, 1×, 2×), whenever it changes.
    var onZoom: (@Sendable (Double) -> Void)?
    private(set) var fps: Int32 = 60
    /// Apple's video HDR: off. It merges exposures across frames and added about 35 ms on an
    /// iPhone 13 (sensor to app 52 ms with it, about 17 without); the delay is felt, on the
    /// phone's own preview too.
    let hdr = OSAllocatedUnfairLock(initialState: false)


    /// Encode only while a computer is connected; the preview alone costs no encoding.
    let sending = OSAllocatedUnfairLock(initialState: false)

    private let sessionQueue = DispatchQueue(label: "camera.session", qos: .userInitiated)
    private let frameQueue = DispatchQueue(label: "camera.frames", qos: .userInteractive)
    private let output = AVCaptureVideoDataOutput()
    private var input: AVCaptureDeviceInput?
    private var encoder: VTCompressionSession?
    private var transfer: VTPixelTransferSession?
    private var pool: CVPixelBufferPool?
    private let wantKeyframe = OSAllocatedUnfairLock(initialState: true)
    private var lastKeyframe: CMTime = .zero
    private var zoomObservation: NSKeyValueObservation?
    /// Where the phone's share of the delay goes, logged every few seconds: sensor to this
    /// callback, this callback to encoded, and whether pictures are being fitted (portrait).
    private var timing = (frames: 0, toApp: 0.0, encode: 0.0, fitted: 0)

    /// The device for a side: the back's combined camera when the phone has one.
    static func device(_ side: Side) -> AVCaptureDevice? {
        switch side {
        case .back:
            return AVCaptureDevice.DiscoverySession(
                deviceTypes: [.builtInTripleCamera, .builtInDualWideCamera, .builtInDualCamera, .builtInWideAngleCamera],
                mediaType: .video, position: .back
            ).devices.first
        case .front:
            return AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front)
        }
    }

    /// The Camera app's lens buttons for a device, as shown zoom values (0.5, 1, 2 or 3).
    static func presets(for device: AVCaptureDevice) -> [Double] {
        let shown = Double(device.displayVideoZoomFactorMultiplier)
        let factors = [1.0] + device.virtualDeviceSwitchOverVideoZoomFactors.map { Double(truncating: $0) }
        return factors.map { ($0 * shown * 10).rounded() / 10 }
    }

    func use(_ device: AVCaptureDevice) {
        sessionQueue.async { [self] in
            session.beginConfiguration()
            session.sessionPreset = .inputPriority
            // The camera leaves the audio session to the microphone: by default each change here
            // (lens, rotation, frame rate) reconfigured it and silenced the microphone for up to
            // a second.
            session.automaticallyConfiguresApplicationAudioSession = false
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
                // Stabilization holds frames back to smooth them: tens of milliseconds a webcam
                // can't spare, and a camera on a stand doesn't shake.
                if c.isVideoStabilizationSupported { c.preferredVideoStabilizationMode = .off }
                if c.isVideoMirroringSupported {
                    c.automaticallyAdjustsVideoMirroring = false
                    c.isVideoMirrored = false
                }
            }
            session.commitConfiguration()
            wantKeyframe.withLock { $0 = true }
            watchPressure(device)
            zoomObservation = device.observe(\.videoZoomFactor, options: [.initial, .new]) { [weak self] d, _ in
                self?.onZoom?(Double(d.videoZoomFactor * d.displayVideoZoomFactorMultiplier))
            }
            if !session.isRunning { session.startRunning() }
        }
    }

    /// Turns the picture sent to the computer to the angle the rotation coordinator gives.
    func setAngle(_ angle: CGFloat) {
        sessionQueue.async { [self] in
            guard let c = output.connection(with: .video), c.isVideoRotationAngleSupported(angle), c.videoRotationAngle != angle else { return }
            c.videoRotationAngle = angle
            wantKeyframe.withLock { $0 = true }
        }
    }

    /// Glides to a shown zoom value (a lens button), as the Camera app does.
    func glide(toShown value: Double) {
        sessionQueue.async { [self] in
            guard let d = input?.device, (try? d.lockForConfiguration()) != nil else { return }
            d.ramp(toVideoZoomFactor: factor(d, shown: value), withRate: 8)
            d.unlockForConfiguration()
        }
    }

    /// Focus and exposure where the person tapped, as the Camera app: a tap keeps both following
    /// the scene from that point; a long press locks them there (AE/AF Lock). `point` is in the
    /// camera's own coordinates (0...1).
    func focus(at point: CGPoint, lock: Bool) {
        sessionQueue.async { [self] in
            guard let d = input?.device, (try? d.lockForConfiguration()) != nil else { return }
            if d.isFocusPointOfInterestSupported {
                d.focusPointOfInterest = point
                let mode: AVCaptureDevice.FocusMode = lock ? .autoFocus : .continuousAutoFocus
                if d.isFocusModeSupported(mode) { d.focusMode = mode }
            }
            if d.isExposurePointOfInterestSupported {
                d.exposurePointOfInterest = point
                let mode: AVCaptureDevice.ExposureMode = lock ? .autoExpose : .continuousAutoExposure
                if d.isExposureModeSupported(mode) { d.exposureMode = mode }
            }
            d.unlockForConfiguration()
        }
    }

    /// Follows a pinch: the zoom at once, within what the camera allows.
    func setZoom(toShown value: Double) {
        sessionQueue.async { [self] in
            guard let d = input?.device, (try? d.lockForConfiguration()) != nil else { return }
            d.videoZoomFactor = factor(d, shown: value)
            d.unlockForConfiguration()
        }
    }

    private func factor(_ d: AVCaptureDevice, shown: Double) -> CGFloat {
        let f = CGFloat(shown) / d.displayVideoZoomFactorMultiplier
        // Beyond 10× digital zoom only blurs.
        return min(max(f, d.minAvailableVideoZoomFactor), min(d.maxAvailableVideoZoomFactor, 10 / d.displayVideoZoomFactorMultiplier))
    }

    func stop() {
        sessionQueue.async { [self] in session.stopRunning() }
    }

    func requestKeyframe() {
        wantKeyframe.withLock { $0 = true }
    }

    /// 1920×1080, full-quality readout first, then the highest rate up to 60; Apple's image
    /// processing as the Camera app applies it, video HDR included.
    private func choose(format device: AVCaptureDevice) {
        let candidates = device.formats.filter {
            let d = CMVideoFormatDescriptionGetDimensions($0.formatDescription)
            return d.width == Self.width && d.height == Self.height
                && CMFormatDescriptionGetMediaSubType($0.formatDescription) == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        }
        let rate = { (f: AVCaptureDevice.Format) in min(60, f.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0) }
        // Full-quality readout before frame rate: binned formats combine sensor pixels and look
        // softer than the Camera app. Among those, the highest rate up to 60.
        // With Apple's Portrait or Studio Light turned on (Video Effects), only formats that can
        // apply them: the effect matters more to the person than 60 frames a second.
        let effects = AVCaptureDevice.isPortraitEffectEnabled || AVCaptureDevice.isStudioLightEnabled
        let usable = effects
            ? candidates.filter { $0.isPortraitEffectSupported || $0.isStudioLightSupported }
            : candidates
        let pool = usable.isEmpty ? candidates : usable
        effectsApplied = effects
        let best = pool.max { a, b in
            (a.isVideoBinned ? 0 : 1, rate(a)) < (b.isVideoBinned ? 0 : 1, rate(b))
        }
        guard let format = best, (try? device.lockForConfiguration()) != nil else { return }
        device.activeFormat = format
        fps = Int32(rate(format))
        frameRates(device, cooler: false)
        // Apple's video HDR off (see hdr).
        device.automaticallyAdjustsVideoHDREnabled = false
        if format.isVideoHDRSupported { device.isVideoHDREnabled = hdr.withLock { $0 } }
        device.unlockForConfiguration()
    }

    /// Up to the format's rate (60) in good light; down to 30 when it's dark, as Apple's cameras
    /// do, so each picture gets twice the light instead of twice the gain (grain). When iOS says
    /// the phone is under heat or power pressure, 30 at most, until it eases.
    private func frameRates(_ device: AVCaptureDevice, cooler: Bool) {
        let fast = cooler ? min(fps, 30) : fps
        device.activeVideoMinFrameDuration = CMTime(value: 1, timescale: fast)
        device.activeVideoMaxFrameDuration = CMTime(value: 1, timescale: min(fast, 30))
    }

    private var pressureObservation: NSKeyValueObservation?
    /// Whether the format was chosen for Apple's video effects.
    private var effectsApplied = false

    /// After the Video Effects picker: a format that can apply what was turned on (or the
    /// fastest again when it was turned off).
    func effectsChanged() {
        sessionQueue.async { [self] in
            let effects = AVCaptureDevice.isPortraitEffectEnabled || AVCaptureDevice.isStudioLightEnabled
            guard effects != effectsApplied, let d = input?.device else { return }
            session.beginConfiguration()
            choose(format: d)
            session.commitConfiguration()
            wantKeyframe.withLock { $0 = true }
            log.notice("video effects \(effects ? "on" : "off", privacy: .public): format chosen again")
        }
    }

    private func watchPressure(_ device: AVCaptureDevice) {
        pressureObservation = device.observe(\.systemPressureState, options: [.new]) { [weak self] d, _ in
            guard let self else { return }
            let level = d.systemPressureState.level
            let cooler = level == .serious || level == .critical || level == .shutdown
            log.notice("system pressure \(level.rawValue, privacy: .public): \(cooler ? "30 fps at most" : "full rate", privacy: .public)")
            self.sessionQueue.async {
                guard (try? d.lockForConfiguration()) != nil else { return }
                self.frameRates(d, cooler: cooler)
                d.unlockForConfiguration()
            }
        }
    }

    /// Encoder settings from best picture down to the setup proven on every phone: a phone's
    /// encoder that refuses one (in low-latency mode some do) steps to the next, so pictures
    /// always flow.
    private static let presets: [(bitRate: Int, qualityFirst: Bool)] = [
        (40_000_000, true), (24_000_000, true), (16_000_000, false),
    ]
    private var preset = 0
    /// Set when an encoded picture comes back with an error: the next picture rebuilds the
    /// encoder one step down.
    private let failed = OSAllocatedUnfairLock(initialState: false)

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
        // The cable carries far more than this: 40 Mbit/s keeps 1080p60 close to what the
        // Camera app shows, and the encoder spends its time on quality (it stays real time).
        let p = Self.presets[min(preset, Self.presets.count - 1)]
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AverageBitRate, value: NSNumber(value: p.bitRate))
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality,
                             value: p.qualityFirst ? kCFBooleanFalse : kCFBooleanTrue)
        VTCompressionSessionPrepareToEncodeFrames(s)
        return s
    }

    /// The frame as the computer gets it: as captured when it's already 1920×1080 (held
    /// sideways), otherwise fitted inside 1920×1080 with black around it, on the graphics
    /// hardware.
    private func landscape(_ image: CVPixelBuffer) -> CVPixelBuffer? {
        if CVPixelBufferGetWidth(image) == Int(Self.width) && CVPixelBufferGetHeight(image) == Int(Self.height) { return image }
        if transfer == nil {
            VTPixelTransferSessionCreate(allocator: nil, pixelTransferSessionOut: &transfer)
            if let t = transfer { VTSessionSetProperty(t, key: kVTPixelTransferPropertyKey_ScalingMode, value: kVTScalingMode_Letterbox) }
        }
        if pool == nil {
            let attributes: [CFString: Any] = [
                kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                kCVPixelBufferWidthKey: Int(Self.width),
                kCVPixelBufferHeightKey: Int(Self.height),
                kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
            ]
            CVPixelBufferPoolCreate(nil, nil, attributes as CFDictionary, &pool)
        }
        guard let transfer, let pool else {
            log.error("no pixel transfer session or pool")
            return nil
        }
        var out: CVPixelBuffer?
        CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out)
        guard let out else {
            log.error("the pool gave no buffer")
            return nil
        }
        let status = VTPixelTransferSessionTransferImage(transfer, from: image, to: out)
        if status != noErr {
            log.error("fitting a \(CVPixelBufferGetWidth(image))x\(CVPixelBufferGetHeight(image)) picture failed: \(status)")
            return nil
        }
        return out
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sample: CMSampleBuffer, from connection: AVCaptureConnection) {
        onFrame?(sample)
        guard sending.withLock({ $0 }), onPicture != nil, let captured = CMSampleBufferGetImageBuffer(sample),
              let image = landscape(captured) else { return }
        if encoder == nil {
            encoder = makeEncoder()
            log.notice("encoder \(self.encoder == nil ? "failed" : "ready", privacy: .public) at preset \(self.preset, privacy: .public)")
        }
        guard let encoder else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(sample)
        let arrived = CACurrentMediaTime()
        timing.frames += 1
        timing.toApp += arrived - pts.seconds
        if image !== captured { timing.fitted += 1 }
        if timing.frames == 300 {
            log.notice("camera timing over 300 frames: sensor to app \(self.timing.toApp / 0.3, privacy: .public) ms, encode \(self.timing.encode / 0.3, privacy: .public) ms, fitted \(self.timing.fitted, privacy: .public)")
            timing = (0, 0, 0, 0)
        }
        // The capture moment on the shared clock: now, less how long ago the sensor took it.
        let capturedUs = Wire.nowUs() - Int64((CACurrentMediaTime() - pts.seconds) * 1_000_000)
        var force = wantKeyframe.withLock { v in defer { v = false }; return v }
        if (pts - lastKeyframe).seconds >= 2 { force = true }
        if force { lastKeyframe = pts }
        let options = force ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        let accepted = VTCompressionSessionEncodeFrame(encoder, imageBuffer: image, presentationTimeStamp: pts, duration: .invalid,
                                        frameProperties: options, infoFlagsOut: nil) { [weak self] status, _, encoded in
            if status != noErr {
                log.error("encoding failed: \(status, privacy: .public)")
                self?.failed.withLock { $0 = true }
            }
            guard status == noErr, let encoded, let self else { return }
            self.timing.encode += CACurrentMediaTime() - arrived
            let (unit, key) = Self.annexB(encoded)
            guard !unit.isEmpty else { return }
            self.onPicture?(Wire.packet(.video, flags: key ? Wire.keyframe : 0, capturedUs: capturedUs, body: unit), key)
        }
        if accepted != noErr || failed.withLock({ v in defer { v = false }; return v }) {
            log.error("the encoder refused a picture (\(accepted, privacy: .public)) at preset \(self.preset, privacy: .public); stepping down")
            VTCompressionSessionInvalidate(encoder)
            self.encoder = nil
            preset = min(preset + 1, Self.presets.count - 1)
            wantKeyframe.withLock { $0 = true }
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
