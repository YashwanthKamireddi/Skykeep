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
    @Namespace private var glass
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
            VStack {
                HStack(alignment: .top) {
                    StatusPill(status: model.status)
                    Spacer()
                    topControls
                }
                .padding(.top, 8)
                if !model.cameraOn {
                    Spacer()
                    Label("Microphone only", systemImage: "mic.fill")
                        .font(.title3.weight(.semibold))
                        .padding(.horizontal, 20)
                        .padding(.vertical, 12)
                        .glassEffect(.regular, in: .capsule)
                }
                Spacer()
                controls
                    .padding(.bottom, 24)
            }
            .padding(.horizontal, 20)
        }
        .animation(.smooth(duration: 0.25), value: model.status)
    }

    private var controls: some View {
        GlassEffectContainer(spacing: 16) {
            HStack(spacing: 16) {
                if model.cameraOn && model.side == .back && model.presets.count > 1 {
                    HStack(spacing: 4) {
                        ForEach(model.presets, id: \.self) { preset in
                            let active = isActive(preset)
                            Button { model.choose(preset: preset) } label: {
                                Text(active ? label(model.zoom) : label(preset))
                                    .font(.footnote.weight(.semibold).monospacedDigit())
                                    .foregroundStyle(active ? Color.yellow : Color.white)
                                    .frame(width: 44, height: 44)
                                    .contentShape(.circle)
                            }
                            .accessibilityLabel("Zoom \(label(preset))")
                            .accessibilityAddTraits(active ? .isSelected : [])
                        }
                    }
                    .padding(.horizontal, 4)
                    .glassEffect(.regular.interactive(), in: .capsule)
                    .glassEffectID("zoom", in: glass)
                }

                if model.cameraOn && model.hasFront {
                    Button(action: model.flip) {
                        Image(systemName: "arrow.triangle.2.circlepath.camera")
                            .font(.title3)
                            .frame(width: 52, height: 52)
                    }
                    .buttonStyle(.glass)
                    .buttonBorderShape(.circle)
                    .accessibilityLabel(model.side == .front ? "Use the back camera" : "Use the front camera")
                }

                Button(action: model.toggleCamera) {
                    Image(systemName: model.cameraOn ? "video.fill" : "video.slash.fill")
                        .font(.title3)
                        .contentTransition(.symbolEffect(.replace))
                        .frame(width: 52, height: 52)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .accessibilityLabel(model.cameraOn ? "Turn the camera off: microphone only" : "Turn the camera on")

                Button(action: model.toggleMic) {
                    Image(systemName: model.micOn ? "mic.fill" : "mic.slash.fill")
                        .font(.title3)
                        .contentTransition(.symbolEffect(.replace))
                        .frame(width: 52, height: 52)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .accessibilityLabel(model.micOn ? "Turn the microphone off" : "Turn the microphone on")
            }
        }
    }

    /// HDR and the microphone mode, small at the top, where the Camera app keeps its own.
    private var topControls: some View {
        GlassEffectContainer(spacing: 8) {
            HStack(spacing: 8) {
                Button(action: model.toggleHDR) {
                    Text("HDR")
                        .font(.caption.weight(.semibold))
                        .strikethrough(!model.hdr)
                        .foregroundStyle(model.hdr ? Color.yellow : Color.white)
                        .frame(width: 40, height: 40)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .disabled(!model.cameraOn)
                .accessibilityLabel(model.hdr ? "Turn HDR off for less delay" : "Turn HDR on for a better picture")

                Button(action: model.showVideoEffects) {
                    Image(systemName: "camera.filters")
                        .font(.footnote.weight(.semibold))
                        .frame(width: 40, height: 40)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .disabled(!model.cameraOn)
                .accessibilityLabel("Video effects: Portrait, Studio Light and more")

                Menu {
                    Picker("Microphone", selection: Binding(get: { model.micVoice }, set: { model.setMicVoice($0) })) {
                        Label("Natural", systemImage: "waveform").tag(false)
                        Label("Voice Isolation", systemImage: "person.wave.2").tag(true)
                    }
                    if model.micVoice {
                        Button("Apple Mic Modes", systemImage: "slider.horizontal.3", action: model.showMicModes)
                    }
                } label: {
                    Image(systemName: model.micVoice ? "person.wave.2" : "waveform")
                        .font(.footnote.weight(.semibold))
                        .frame(width: 40, height: 40)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .accessibilityLabel(model.micVoice ? "Microphone: Voice Isolation" : "Microphone: Natural")
            }
        }
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

struct StatusPill: View {
    let status: Model.Status

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
            Text(text)
                .font(.subheadline.weight(.medium))
                .lineLimit(2)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
        .glassEffect(.regular, in: .capsule)
        .accessibilityElement(children: .combine)
    }

    private var text: String {
        switch status {
        case .starting: "Starting the camera"
        case .needsAccess: "Allow the camera in Settings › Skykeep"
        case .waiting: "Connect to your computer with a cable"
        case .connected: "Your computer is using this camera"
        case .failed(let why): why
        }
    }

    private var color: Color {
        switch status {
        case .connected: .green
        case .failed, .needsAccess: .red
        default: .secondary
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
