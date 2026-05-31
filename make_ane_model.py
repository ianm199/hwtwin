"""Builds a convolution-heavy CoreML model that the Apple Neural Engine will
execute, so the mapping probe can see which sensors respond to ANE load. Saved
as an ML Program (.mlpackage) targeting CPU+NE."""

import coremltools as ct
import numpy as np
from coremltools.converters.mil import Builder as mb

C = 128
WEIGHT = np.random.rand(C, C, 3, 3).astype(np.float32)


@mb.program(input_specs=[mb.TensorSpec(shape=(1, C, 96, 96))])
def prog(x):
    for _ in range(12):
        x = mb.conv(x=x, weight=WEIGHT, pad_type="same")
        x = mb.relu(x=x)
    return x


model = ct.convert(
    prog,
    compute_units=ct.ComputeUnit.CPU_AND_NE,
    minimum_deployment_target=ct.target.macOS13,
)
model.save("ane_model.mlpackage")
print("saved ane_model.mlpackage")
