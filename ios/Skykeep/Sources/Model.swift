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
    /// Off: the iPhone is only the computer's microphone (less heat and battery).
    private(set) var cameraOn = true

    let camera = Camera()
    let pip = PictureInPicture()
    private let microphone = Microphone()
    private let link = Link()
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var rotation: AVCaptureDevice.RotationCoordinator?
    private var rotationWatch: [NSKeyValueObservation] = []
    private var pinchStart: Double = 1
    /// iOS stopped the camera (the phone locked, or another app took the screen with no
    /// picture in picture). The computer is told, so it shows black and says why.
    private(set) var cameraPaused = false
    private var interruptions: [NSObjectProtocol] = []

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
            log.notice("link \(String(describing: state), privacy: .public)")
            Task { @MainActor in self?.linkChanged(state) }
        }
        guard use(.back) || use(.front) else { status = .failed("This iPhone has no camera Skykeep can use."); return }
        if micOn { startMicrophone() }
        pip.attach()
        link.start(greeting: greeting())
        let center = NotificationCenter.default
        interruptions = [
            center.addObserver(forName: AVCaptureSession.wasInterruptedNotification, object: camera.session, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.setCameraPaused(true) }
            },
            center.addObserver(forName: AVCaptureSession.interruptionEndedNotification, object: camera.session, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.setCameraPaused(false) }
            },
        ]
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
        // 1× when the camera opens (the combined camera starts at its widest, 0.5×): set at
        // once, not glided, since a glide before the camera runs is dropped.
        if side == .back { camera.setZoom(toShown: 1) }
        let microphone = self.microphone
        audioQueue.async { microphone.face(side) }
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

    /// The audio session and engine are started and stopped here, off the main thread: they
    /// block for a tenth of a second or more, and on the main thread every touch waited.
    private let audioQueue = DispatchQueue(label: "microphone", qos: .userInitiated)

    private func startMicrophone() {
        let microphone = self.microphone, side = self.side
        audioQueue.async {
            do {
                try microphone.start()
                // The session exists now: the microphone on the camera's side.
                microphone.face(side)
            } catch {
                log.error("microphone failed to start: \(error.localizedDescription, privacy: .public)")
                Task { @MainActor [weak self] in self?.micOn = false }
            }
        }
    }

    func toggleCamera() {
        cameraOn.toggle()
        if cameraOn {
            if let device = Camera.device(side) { camera.use(device) }
        } else {
            camera.stop()
        }
        link.send(cameraState())
        link.setGreeting(greeting(), sendNow: false)
    }

    /// Where the person last tapped (in the preview), and whether it's locked: the yellow square.
    private(set) var focusMark: (point: CGPoint, locked: Bool, id: Int)?

    func focus(atLayerPoint point: CGPoint, lock: Bool) {
        guard cameraOn, let layer = previewLayer else { return }
        camera.focus(at: layer.captureDevicePointConverted(fromLayerPoint: point), lock: lock)
        let id = (focusMark?.id ?? 0) + 1
        focusMark = (point, lock, id)
        if !lock {
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: .seconds(1.2))
                if self?.focusMark?.id == id { self?.focusMark = nil }
            }
        }
    }





    func toggleMic() {
        micOn.toggle()
        if micOn {
            startMicrophone()
        } else {
            let microphone = self.microphone
            audioQueue.async { microphone.stop() }
        }
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
        rotationWatch.append(r.observe(\.videoRotationAngleForHorizonLevelPreview, options: [.initial, .new]) { [weak self] r, _ in
            let angle = r.videoRotationAngleForHorizonLevelPreview
            Task { @MainActor in
                if let c = self?.previewLayer?.connection, c.isVideoRotationAngleSupported(angle) { c.videoRotationAngle = angle }
            }
        })
    }

    private func setCameraPaused(_ paused: Bool) {
        cameraPaused = paused
        log.notice("camera \(paused ? "paused by iOS" : "live again", privacy: .public)")
        link.send(cameraState())
        // A computer that connects later is told the state first thing.
        link.setGreeting(greeting(), sendNow: false)
        if !paused { camera.requestKeyframe() }
    }

    private func cameraState() -> Data {
        Wire.message(.control, ["camera": !cameraOn ? "off" : cameraPaused ? "paused" : "live"])
    }

    private func greeting() -> [Data] {
        [
            Wire.message(.hello, ["version": Wire.version, "name": "iPhone"]),
            Wire.message(.videoFormat, ["codec": "h264", "width": Int(Camera.width), "height": Int(Camera.height), "fps": Int(camera.fps)]),
            cameraState(),
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
