"""Writes a copy of a TFLite model with its sparse constant tensors stored dense.

Each DENSIFY operator turns a sparse constant into a dense one. TFLite's interpreter
computes those dense values; this script stores them in the sparse tensor's place, removes
the DENSIFY operators and points their consumers at the now dense tensor. The values are
unchanged, so the copy computes exactly what the original does, which the script checks on
random inputs. Usage:
    python -I densify.py model.tflite dense.tflite
"""
import sys

import flatbuffers
import numpy as np
import tensorflow as tf
from tensorflow.lite.python import schema_py_generated as S

source, target = sys.argv[1], sys.argv[2]
buf = open(source, "rb").read()
model = S.ModelT.InitFromObj(S.Model.GetRootAsModel(buf, 0))
graph = model.subgraphs[0]
densify = [i for i, c in enumerate(model.operatorCodes)
           if max(c.builtinCode, c.deprecatedBuiltinCode) == S.BuiltinOperator.DENSIFY]
ops = [op for op in graph.operators if op.opcodeIndex in densify]

resolver = tf.lite.experimental.OpResolverType.BUILTIN_WITHOUT_DEFAULT_DELEGATES
original = tf.lite.Interpreter(model_path=source, experimental_op_resolver_type=resolver,
                               experimental_preserve_all_tensors=True)
original.allocate_tensors()
inp = original.get_input_details()[0]
original.set_tensor(inp["index"], np.zeros(inp["shape"], dtype=np.float32))
original.invoke()

renamed = {}
for op in ops:
    sparse, dense = int(op.inputs[0]), int(op.outputs[0])
    values = original.get_tensor(dense)
    tensor = graph.tensors[sparse]
    tensor.sparsity = None
    tensor.shape = np.array(values.shape, dtype=np.int32)
    model.buffers[tensor.buffer].data = np.frombuffer(values.tobytes(), dtype=np.uint8)
    renamed[dense] = sparse
graph.operators = [op for op in graph.operators if op.opcodeIndex not in densify]
for op in graph.operators:
    op.inputs = np.array([renamed.get(int(i), int(i)) for i in op.inputs], dtype=np.int32)

builder = flatbuffers.Builder(1 << 22)
builder.Finish(model.Pack(builder), file_identifier=b"TFL3")
open(target, "wb").write(builder.Output())

# The dense copy must compute exactly what the original computes.
copy = tf.lite.Interpreter(model_path=target, experimental_op_resolver_type=resolver)
copy.allocate_tensors()
fresh = tf.lite.Interpreter(model_path=source, experimental_op_resolver_type=resolver)
fresh.allocate_tensors()
rng = np.random.default_rng(3)
worst = 0.0
for trial in range(3):
    value = rng.uniform(-1.0, 1.0, size=inp["shape"]).astype(np.float32)
    for interpreter in (fresh, copy):
        interpreter.set_tensor(interpreter.get_input_details()[0]["index"], value)
        interpreter.invoke()
    for a, b in zip(fresh.get_output_details(), copy.get_output_details()):
        worst = max(worst, float(np.max(np.abs(fresh.get_tensor(a["index"]) - copy.get_tensor(b["index"])))))
print(f"densified {len(ops)} tensors; largest output difference from the original: {worst}")
