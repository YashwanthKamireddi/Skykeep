import AVFoundation
import Observation
import UIKit

/// What the screen shows and what it can change: the link's state, the camera, the zoom, the
/// microphone.
@MainActor
@Observable
final class Model {
    enum Status: Equatable { case starting, needsAccess, waiting, connected, failed(String) }

    private(set) var status: Status = .starting
    private(set) var side: Camera.Side = .back
    /// The back camera's lens buttons, as the Camera app shows them (0.5, 1, 2…).
    private(set) var presets: [Double] = []
    /// The zoom now, as shown (1.4 while pinching between 1 and 2).
    private(set) var zoom: Double = 1
    private(set) var hasFront = false
    private(set) var micOn = true

    let camera = Camera()
    let pip = PictureInPicture()
    private let microphone = Microphone()
    private let link = Link()
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var rotation: AVCaptureDevice.RotationCoordinator?
    private var rotationWatch: [NSKeyValueObservation] = []
    private var pinchStart: Double = 1

    func start() async {
        let cameraOK = await AVCaptureDevice.requestAccess(for: .video)
        let micOK = await AVAudioApplication.requestRecordPermission()
        guard cameraOK else { status = .needsAccess; return }
        micOn = micOK
        hasFront = Camera.device(.front) != nil

        let camera = self.camera, link = self.link, pip = self.pip, microphone = self.microphone
        camera.onPicture = { packet, key in link.sendPicture(packet, keyframe: key) }
        camera.onFrame = { sample in pip.show(sample) }
        camera.onZoom = { [weak self] z in Task { @MainActor in self?.zoom = z } }
        link.onKeyframeNeeded = { camera.requestKeyframe() }
        microphone.onSound = { packet in link.sendSound(packet) }
        link.onState = { [weak self] state in
            camera.sending.withLock { $0 = state == .connected }
            Task { @MainActor in self?.linkChanged(state) }
        }
        guard use(.back) || use(.front) else { status = .failed("This iPhone has no camera Skykeep can use."); return }
        if micOn { try? microphone.start() }
        pip.attach()
        link.start(greeting: greeting())
    }

    /// The preview on screen, so the rotation coordinator can turn it and the picture together.
    func attach(preview: AVCaptureVideoPreviewLayer) {
        previewLayer = preview
        if let device = Camera.device(side) { watchRotation(of: device) }
    }

    @discardableResult
    private func use(_ side: Camera.Side) -> Bool {
        guard let device = Camera.device(side) else { return false }
        self.side = side
        presets = side == .back ? Camera.presets(for: device) : []
        camera.use(device)
        if side == .back, let one = presets.first(where: { $0 == 1 }) { camera.glide(toShown: one) }
        watchRotation(of: device)
        return true
    }

    func flip() {
        use(side == .back ? .front : .back)
    }

    func choose(preset: Double) {
        camera.glide(toShown: preset)
    }

    func pinchBegan() { pinchStart = zoom }

    func pinch(by scale: Double) {
        guard side == .back else { return }
        camera.setZoom(toShown: pinchStart * scale)
    }

    func toggleMic() {
        micOn.toggle()
        if micOn { try? microphone.start() } else { microphone.stop() }
    }

    /// Apple's rotation coordinator: the angle that keeps the horizon level for the picture
    /// sent and for the preview, followed as the phone turns.
    private func watchRotation(of device: AVCaptureDevice) {
        guard let previewLayer else { return }
        rotationWatch = []
        let r = AVCaptureDevice.RotationCoordinator(device: device, previewLayer: previewLayer)
        rotation = r
        let camera = self.camera
        rotationWatch.append(r.observe(\.videoRotationAngleForHorizonLevelCapture, options: [.initial, .new]) { r, _ in
            camera.setAngle(r.videoRotationAngleForHorizonLevelCapture)
        })
        rotationWatch.append(r.observe(\.videoRotationAngleForHorizonLevelPreview, options: [.initial, .new]) { [weak previewLayer] r, _ in
            let angle = r.videoRotationAngleForHorizonLevelPreview
            Task { @MainActor in
                if let c = previewLayer?.connection, c.isVideoRotationAngleSupported(angle) { c.videoRotationAngle = angle }
            }
        })
    }

    private func greeting() -> [Data] {
        [
            Wire.message(.hello, ["version": Wire.version, "name": "iPhone"]),
            Wire.message(.videoFormat, ["codec": "h264", "width": Int(Camera.width), "height": Int(Camera.height), "fps": Int(camera.fps)]),
        ]
    }

    private func linkChanged(_ state: Link.State) {
        switch state {
        case .waiting: status = .waiting
        case .connected: status = .connected
        case .failed(let why): status = .failed(why)
        }
        UIApplication.shared.isIdleTimerDisabled = state == .connected
    }
}
