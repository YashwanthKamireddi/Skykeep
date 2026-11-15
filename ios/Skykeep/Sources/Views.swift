import AVFoundation
import SwiftUI

@main
struct SkykeepApp: App {
    @State private var model = Model()

    var body: some Scene {
        WindowGroup {
            CameraScreen(model: model)
                .task { await model.start() }
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
            VStack {
                StatusPill(status: model.status)
                    .padding(.top, 8)
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
                if model.side == .back && model.presets.count > 1 {
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

                if model.hasFront {
                    Button(action: model.flip) {
                        Image(systemName: "arrow.triangle.2.circlepath.camera")
                            .font(.title3)
                            .frame(width: 52, height: 52)
                    }
                    .buttonStyle(.glass)
                    .buttonBorderShape(.circle)
                    .accessibilityLabel(model.side == .front ? "Use the back camera" : "Use the front camera")
                }

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
        pip.layer.frame = CGRect(x: 0, y: 0, width: 2, height: 2)
        pip.layer.opacity = 0.01
        view.layer.addSublayer(pip.layer)
        return view
    }

    func updateUIView(_ view: PreviewView, context: Context) {}
}

final class PreviewView: UIView {
    override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
    var preview: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
}
