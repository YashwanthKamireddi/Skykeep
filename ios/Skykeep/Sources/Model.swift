import AVFoundation
import Observation
import UIKit

/// What the screen shows and what it can change: the link's state, the lens, the microphone.
@MainActor
@Observable
final class Model {
    enum Status: Equatable { case starting, needsAccess, waiting, connected, failed(String) }

    private(set) var status: Status = .starting
    private(set) var lenses: [Camera.Lens] = []
    private(set) var lens: Camera.Lens?
    private(set) var micOn = true

    let camera = Camera()
    let pip = PictureInPicture()
    private let microphone = Microphone()
    private let link = Link()

    func start() async {
        let cameraOK = await AVCaptureDevice.requestAccess(for: .video)
        let micOK = await AVAudioApplication.requestRecordPermission()
        guard cameraOK else { status = .needsAccess; return }
        micOn = micOK

        lenses = Camera.lenses()
        lens = lenses.first { !$0.front && $0.label == "1×" } ?? lenses.first
        guard let lens else { status = .failed("This iPhone has no camera Skykeep can use."); return }

        let camera = self.camera, link = self.link, pip = self.pip, microphone = self.microphone
        camera.onPicture = { packet, key in link.sendPicture(packet, keyframe: key) }
        camera.onFrame = { sample in pip.show(sample) }
        link.onKeyframeNeeded = { camera.requestKeyframe() }
        microphone.onSound = { packet in link.sendSound(packet) }
        link.onState = { [weak self] state in
            camera.sending.withLock { $0 = state == .connected }
            Task { @MainActor in self?.linkChanged(state) }
        }
        camera.configure(lens: lens)
        if micOn { try? microphone.start() }
        pip.attach()
        link.start(greeting: greeting())
    }

    func choose(_ lens: Camera.Lens) {
        guard lens != self.lens else { return }
        self.lens = lens
        camera.configure(lens: lens)
        link.setGreeting(greeting())
    }

    func toggleMic() {
        micOn.toggle()
        if micOn { try? microphone.start() } else { microphone.stop() }
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
