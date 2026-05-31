"""Drives the Apple Neural Engine by running the conv model in a tight loop for
a given number of seconds. Usage: ane_stress.py <seconds>"""

import sys
import time

import coremltools as ct
import numpy as np

secs = float(sys.argv[1]) if len(sys.argv) > 1 else 30.0
model = ct.models.MLModel("ane_model.mlpackage", compute_units=ct.ComputeUnit.CPU_AND_NE)
input_name = model.get_spec().description.input[0].name
x = np.random.rand(1, 128, 96, 96).astype(np.float32)

end = time.time() + secs
count = 0
while time.time() < end:
    model.predict({input_name: x})
    count += 1
print(f"ane_stress: {count} inferences over {secs}s")
