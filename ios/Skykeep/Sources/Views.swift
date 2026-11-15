import AVFoundation
import SwiftUI

@main
struct SkykeepApp: App {
    @State private var model = Model()
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            CameraScreen(model: model)
                .task { await model.start() }
                .onChange(of: phase) { _, now in
                    let active = now == .active
                    model.pip.onScreen.withLock { $0 = active }
                    if active { model.pip.appInFront() }
                    // Back from Control Center or the Video Effects picker.
                    if active { model.camera.effectsChanged() }
                }
                .preferredColorScheme(.dark)
        }
    }
}

/// One screen, like the Camera app: what the computer sees, edge to edge, and the few
/// controls on glass over it.
struct CameraScreen: View {
    let model: Model
    @State private var pinching = false

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            Preview(session: model.camera.session, pip: model.pip, model: model)
                .ignoresSafeArea()
                .gesture(
                    MagnifyGesture()
                        .onChanged { value in
                            if !pinching { model.pinchBegan(); pinching = true }
                            model.pinch(by: value.magnification)
                        }
                        .onEnded { _ in pinching = false }
                )
            if let mark = model.focusMark {
                FocusSquare(locked: mark.locked)
                    .position(mark.point)
                    .id(mark.id)
                    .allowsHitTesting(false)
            }
            // Camera turned off: the preview layer keeps its last frame, which read as a frozen
            // app. Covered instead, saying what is still on.
            if !model.cameraOn {
                Color.black.ignoresSafeArea()
                VStack(spacing: 10) {
                    Image(systemName: "video.slash")
                        .font(.system(size: 40, weight: .light))
                        .foregroundStyle(.secondary)
                    Text("Camera off").font(.title3.weight(.semibold))
                    Text(model.micOn ? "The microphone is still on." : "The microphone is off too.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                .multilineTextAlignment(.center)
                .allowsHitTesting(false)
                .transition(.opacity)
            }
            if model.status == .connected {
                VStack {
                    LiveBadge(cameraOn: model.cameraOn)
                        .padding(.top, 8)
                        .allowsHitTesting(false)
                    Spacer()
                }
            } else {
                Waiting(status: model.status)
                    .padding(.horizontal, 40)
                    .allowsHitTesting(false)
                    .transition(.opacity)
            }
            VStack(spacing: 20) {
                Spacer()
                if model.cameraOn && model.side == .back && model.presets.count > 1 {
                    zoom.transition(.opacity)
                }
                controls
            }
            .padding(.bottom, 28)
        }
        .animation(.smooth(duration: 0.25), value: model.status)
        .animation(.smooth(duration: 0.2), value: model.cameraOn)
        .sensoryFeedback(.selection, trigger: model.cameraOn)
        .sensoryFeedback(.selection, trigger: model.micOn)
        .sensoryFeedback(.selection, trigger: model.side)
    }

    /// One size for every round button, the Camera app's: 60 pt, on glass.
    private static let button: CGFloat = 60

    /// The lenses, as the Camera app has them: a small capsule above the buttons, the one in
    /// use in yellow with the zoom it's at.
    private var zoom: some View {
        HStack(spacing: 2) {
            ForEach(model.presets, id: \.self) { preset in
                let active = isActive(preset)
                Button { model.choose(preset: preset) } label: {
                    Text(active ? label(model.zoom) : label(preset))
                        .font(.caption.weight(.semibold).monospacedDigit())
                        .foregroundStyle(active ? Color.yellow : Color.white)
                        .frame(width: 40, height: 40)
                        .contentShape(.circle)
                }
                .accessibilityLabel("Zoom \(label(preset))")
                .accessibilityAddTraits(active ? .isSelected : [])
            }
        }
        .padding(4)
        .glassEffect(.regular.interactive(), in: .capsule)
    }

    private var controls: some View {
        GlassEffectContainer(spacing: 28) {
            HStack(spacing: 28) {
                if model.cameraOn && model.hasFront {
                    round("arrow.triangle.2.circlepath", action: model.flip)
                        .accessibilityLabel(model.side == .front ? "Use the back camera" : "Use the front camera")
                }
                round(model.cameraOn ? "video.fill" : "video.slash.fill", action: model.toggleCamera)
                    .accessibilityLabel(model.cameraOn ? "Turn the camera off: microphone only" : "Turn the camera on")
                round(model.micOn ? "mic.fill" : "mic.slash.fill", action: model.toggleMic)
                    .accessibilityLabel(model.micOn ? "Turn the microphone off" : "Turn the microphone on")
            }
        }
    }

    private func round(_ symbol: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 22, weight: .medium))
                .contentTransition(.symbolEffect(.replace))
                .frame(width: Self.button, height: Self.button)
        }
        .buttonStyle(.glass)
        .buttonBorderShape(.circle)
    }

    /// The button for the lens now in use: the highest preset at or under the zoom.
    private func isActive(_ preset: Double) -> Bool {
        let current = model.presets.last { $0 <= model.zoom + 0.05 } ?? model.presets.first
        return current == preset
    }

    private func label(_ value: Double) -> String {
        let rounded = (value * 10).rounded() / 10
        return rounded == rounded.rounded() ? "\(Int(rounded))×" : String(format: "%.1f×", rounded)
    }
}

/// Connected: a small badge at the top, as the Camera app marks what it's doing; nothing more.
struct LiveBadge: View {
    let cameraOn: Bool

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(.green).frame(width: 6, height: 6)
            Text(cameraOn ? "Live" : "Microphone only")
                .font(.caption.weight(.semibold))
        }
        .padding(.horizontal, 10)
        .frame(height: 26)
        .glassEffect(.regular, in: .capsule)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(cameraOn ? "Connected: the computer sees and hears this iPhone" : "Connected: microphone only")
    }
}

/// Not connected yet: what to do, in the middle of the screen, in a sentence.
struct Waiting: View {
    let status: Model.Status

    var body: some View {
        VStack(spacing: 10) {
            if status == .starting {
                ProgressView().controlSize(.large)
            } else {
                Image(systemName: symbol)
                    .font(.system(size: 40, weight: .light))
                    .foregroundStyle(.secondary)
                    .padding(.bottom, 4)
            }
            Text(title).font(.title3.weight(.semibold))
            if let detail {
                Text(detail).font(.subheadline).foregroundStyle(.secondary)
            }
        }
        .multilineTextAlignment(.center)
        .accessibilityElement(children: .combine)
    }

    private var symbol: String {
        switch status {
        case .needsAccess: "video.slash"
        case .failed: "exclamationmark.triangle"
        default: "cable.connector"
        }
    }

    private var title: String {
        switch status {
        case .starting: "Starting"
        case .needsAccess: "Allow the camera"
        case .failed: "Something went wrong"
        default: "Plug into your computer"
        }
    }

    private var detail: String? {
        switch status {
        case .needsAccess: "Turn on Camera and Microphone for Skykeep in Settings."
        case .failed(let why): why
        case .waiting: "Use a cable, and turn on iPhone in Skykeep's Settings on the computer."
        default: nil
        }
    }
}

/// The live preview, with the hidden picture-in-picture layer beside it (iOS needs that layer
/// on screen to float it when another app comes in front).
struct Preview: UIViewRepresentable {
    let session: AVCaptureSession
    let pip: PictureInPicture
    let model: Model

    func makeUIView(context: Context) -> PreviewView {
        let view = PreviewView()
        view.preview.session = session
        view.preview.videoGravity = .resizeAspect
        model.attach(preview: view.preview)
        view.addGestureRecognizer(UITapGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.tap)))
        let press = UILongPressGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.press))
        press.minimumPressDuration = 0.6
        view.addGestureRecognizer(press)
        pip.layer.frame = CGRect(x: 0, y: 0, width: 2, height: 2)
        pip.layer.opacity = 0.01
        view.layer.addSublayer(pip.layer)
        return view
    }

    func updateUIView(_ view: PreviewView, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(model: model) }

    /// Tap to focus and expose there; long press to lock both (AE/AF Lock), as the Camera app.
    final class Coordinator: NSObject {
        let model: Model
        init(model: Model) { self.model = model }

        @MainActor @objc func tap(_ g: UITapGestureRecognizer) {
            model.focus(atLayerPoint: g.location(in: g.view), lock: false)
        }

        @MainActor @objc func press(_ g: UILongPressGestureRecognizer) {
            if g.state == .began { model.focus(atLayerPoint: g.location(in: g.view), lock: true) }
        }
    }
}

final class PreviewView: UIView {
    override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
    var preview: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
}

/// The Camera app's yellow focus square; with "AE/AF LOCK" above it when locked.
struct FocusSquare: View {
    let locked: Bool
    @State private var shrunk = false

    var body: some View {
        VStack(spacing: 6) {
            if locked {
                Text("AE/AF LOCK")
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(.black)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Color.yellow, in: .rect(cornerRadius: 3))
            }
            Rectangle()
                .stroke(Color.yellow, lineWidth: 1.5)
                .frame(width: 72, height: 72)
                .scaleEffect(shrunk ? 1 : 1.35)
        }
        .onAppear { withAnimation(.snappy(duration: 0.25)) { shrunk = true } }
        .accessibilityHidden(true)
    }
}
