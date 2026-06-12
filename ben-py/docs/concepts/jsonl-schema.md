# JSONL input schema

The whole-file codec helpers read JSON Lines: one JSON object per line, each with an
`assignment` field.

```json
{"assignment": [1, 1, 2, 2], "sample": 1}
{"assignment": [1, 2, 2, 2], "sample": 2}
```

Use this format when a sampler has already written plans to disk and you want to convert the
complete file to BEN or XBEN in one call.

## Required field

`assignment`
: A list of integer district ids. Every line in one file must describe the same fixed graph
  order, so every `assignment` should have the same length.

```python
import json

with open("plans.jsonl") as handle:
    first = json.loads(next(handle))

assert isinstance(first["assignment"], list)
assert all(isinstance(value, int) for value in first["assignment"])
```

## Optional fields

The codec ignores every field except `assignment`. Fields like `sample`, `score`,
`cut_edges`, or sampler metadata may be useful in the source file, but they are not stored in
plain `.ben` or `.xben` streams.

If those fields need to travel with the compressed ensemble, use a `.bendl` file and store
them as metadata or custom assets.

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("jsonl-contract.bendl", overwrite=True)
encoder.add_metadata({"source": "plans.jsonl", "assignment_field": "assignment"})
with encoder.stream() as stream:
    stream.write([1, 1, 2, 2])
```

## Validation before conversion

For large files, validate the cheap structural invariants before starting an expensive
conversion:

```python
import json

expected_length = None

with open("plans.jsonl") as handle:
    for line_number, line in enumerate(handle, start=1):
        row = json.loads(line)
        assignment = row["assignment"]

        if expected_length is None:
            expected_length = len(assignment)

        if len(assignment) != expected_length:
            raise ValueError(f"line {line_number}: assignment length changed")

        if not all(isinstance(value, int) and 0 <= value <= 65535 for value in assignment):
            raise ValueError(f"line {line_number}: assignment values must be 16-bit district ids")
```

That check does not prove the assignments match the intended graph order. It only verifies
that the JSONL is structurally safe to encode. Node order is covered in
[The data contract](data-model.md).

## Conversion

```python
from binary_ensemble import encode_jsonl_to_ben, encode_jsonl_to_xben

encode_jsonl_to_ben("plans.jsonl", "plans.ben", overwrite=True)
encode_jsonl_to_xben("plans.jsonl", "plans.xben", overwrite=True)
```

Use `encode_jsonl_to_ben()` when you plan to keep working with the ensemble. Use
`encode_jsonl_to_xben()` when the output is immediately going to archive or transfer.

