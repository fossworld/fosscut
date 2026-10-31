"""Converts a MediaPipe TFLite model to ONNX with tf2onnx, then checks the ONNX model
against the TFLite model on random inputs.

TensorFlow 2.15's TFLite interpreter crashes in get_tensor_details() on the pose detector
when the default XNNPACK delegate is applied, so the interpreter tf2onnx uses for tensor
shapes is opened without default delegates. The conversion itself is tf2onnx's.

Usage:
    python -I convert_pose.py model.tflite out.onnx
"""
import sys

import numpy as np
import tensorflow as tf

_Interpreter = tf.lite.Interpreter


def plain_interpreter(*args, **kwargs):
    kwargs.setdefault(
        "experimental_op_resolver_type",
        tf.lite.experimental.OpResolverType.BUILTIN_WITHOUT_DEFAULT_DELEGATES,
    )
    return _Interpreter(*args, **kwargs)


class NoDetails(_Interpreter):
    """An interpreter whose get_tensor_details() raises instead of crashing, so tf2onnx
    falls back to the tensor shapes stored in the model (it catches the exception)."""

    def get_tensor_details(self):
        raise RuntimeError("tensor details skipped")


STORED_SHAPES = "--stored-shapes" in sys.argv


def converter_interpreter(*args, **kwargs):
    if STORED_SHAPES:
        return NoDetails(*args, **kwargs)
    return plain_interpreter(*args, **kwargs)


tf.lite.Interpreter = converter_interpreter

import onnxruntime  # noqa: E402
import tf2onnx.convert  # noqa: E402

source, target = sys.argv[1], sys.argv[2]
sys.argv = ["tf2onnx", "--tflite", source, "--output", target, "--opset", "17"]
try:
    tf2onnx.convert.main()
except SystemExit:
    pass

# Compare on random inputs in the range each model expects.
interpreter = plain_interpreter(model_path=source)
interpreter.allocate_tensors()
inputs = interpreter.get_input_details()
outputs = interpreter.get_output_details()
session = onnxruntime.InferenceSession(target, providers=["CPUExecutionProvider"])
rng = np.random.default_rng(7)
worst = {}
for trial in range(3):
    feeds = {}
    for detail, onnx_input in zip(inputs, session.get_inputs()):
        value = rng.uniform(-1.0, 1.0, size=detail["shape"]).astype(np.float32)
        interpreter.set_tensor(detail["index"], value)
        feeds[onnx_input.name] = value
    interpreter.invoke()
    expected = {d["name"]: interpreter.get_tensor(d["index"]) for d in outputs}
    got = dict(zip([o.name for o in session.get_outputs()], session.run(None, feeds)))
    for (name, want), (onnx_name, have) in zip(expected.items(), got.items()):
        scale = float(np.max(np.abs(want))) + 1e-6
        error = float(np.max(np.abs(want - have))) / scale
        worst[onnx_name] = max(worst.get(onnx_name, 0.0), error)
print("inputs:", [(i.name, i.shape, i.type) for i in session.get_inputs()])
print("outputs:", [(o.name, o.shape, o.type) for o in session.get_outputs()])
print("largest error relative to each output's range, TFLite vs ONNX:", worst)
