# Model conversions

FossCut runs its vision and voice models through ONNX Runtime ([ADR-0033](../../docs/adr/0033-run-local-vision-and-voice-models-with-onnx-runtime.md)). Most models are published in ONNX form by their authors. Google publishes MediaPipe Pose Landmarker only as TFLite, so FossCut converts it with these scripts and hosts the result as assets of the `models-v1` release. Anyone can repeat the conversion and compare checksums.

## MediaPipe Pose Landmarker (full)

- **Source:** `https://storage.googleapis.com/mediapipe-models/pose_landmarker/pose_landmarker_full/float16/1/pose_landmarker_full.task`, Apache-2.0, SHA-256 `5134a3aad27a58b93da0088d431f366da362b44e3ccfbe3462b3827a839011b1`.
- **Tools:** Python 3.10 with TensorFlow 2.15.1, tf2onnx 1.16.1, onnx 1.16.2, onnxruntime 1.19.2, flatbuffers 25.12.19 and numpy 1.26.4.

Steps:

1. Unpack the `.task` file, a ZIP archive holding `pose_detector.tflite` and `pose_landmarks_detector.tflite`.
2. The detector stores 38 weight tensors in a sparse form that tf2onnx cannot read. `densify.py` stores the values TFLite computes from them densely instead. The values are unchanged, and the script checks the copy against the original on random inputs:

   ```bash
   python -I densify.py pose_detector.tflite pose_detector_dense.tflite
   ```

3. Convert both models, each compared with its TFLite original on random inputs:

   ```bash
   python -I convert_pose.py pose_detector_dense.tflite mediapipe-pose-full-detector.onnx
   python -I convert_pose.py pose_landmarks_detector.tflite mediapipe-pose-full-landmarks.onnx
   ```

The detector's outputs match TFLite within 1.2e-6 of their range and the landmarks within 0.01 pixels. Two conversions give identical bytes:

| File | Size | SHA-256 |
|---|---|---|
| `mediapipe-pose-full-detector.onnx` | 11,925,808 | `d36c4aecbfea4ed0a9a5cfb1905569437f7e4b777f5921d60e5dc5d4f154e323` |
| `mediapipe-pose-full-landmarks.onnx` | 12,765,479 | `f29febba853796d52f94948632f138c53c617deb09f8539d31b1a4a14b023260` |
